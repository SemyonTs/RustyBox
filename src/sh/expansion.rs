// =============================================================================
// sh expansion — alias, tilde, variable, command substitution, and glob
// expansion.  Each phase respects POSIX quoting semantics.
// =============================================================================
use crate::sh::globals::*;
use crate::sh::parser::{Token, find_closing_paren};
use crate::sh::signals::{acquire_child_slot, release_child_slot};
use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::CStr;
use std::io;
use std::os::unix::io::RawFd;
use std::path::Path;
use std::sync::atomic::Ordering;

// Forward declaration of exec entry points used by command substitution.
// These are implemented in crate::sh::exec.
use crate::sh::exec::run_single_command_list;

// -----------------------------------------------------------------------------
// Alias expansion with cycle detection.
// -----------------------------------------------------------------------------
pub fn expand_aliases_struct(tokens: Vec<Token>, aliases: &HashMap<String, String>) -> Vec<Token> {
    let mut expanded_set = HashSet::new();
    expand_aliases_recursive(tokens, aliases, &mut expanded_set)
}

fn expand_aliases_recursive(
    mut tokens: Vec<Token>,
    aliases: &HashMap<String, String>,
    expanded: &mut HashSet<String>,
) -> Vec<Token> {
    if tokens.is_empty() {
        return tokens;
    }
    // Only expand the first token if it is not quoted.
    if !tokens[0].is_single_quoted && !tokens[0].is_double_quoted && !tokens[0].is_escaped {
        let name = &tokens[0].value;
        if !expanded.contains(name) {
            if let Some(expansion) = aliases.get(name) {
                expanded.insert(name.clone());
                let mut new_tokens = crate::sh::parser::tokenize(expansion);
                if !new_tokens.is_empty() {
                    // Recursively expand the new first token.
                    new_tokens.extend_from_slice(&tokens[1..]);
                    return expand_aliases_recursive(new_tokens, aliases, expanded);
                }
            }
        }
    }
    tokens
}

/// Legacy alias expander for string vectors.
pub fn expand_aliases(tokens: Vec<String>, aliases: &HashMap<String, String>) -> Vec<String> {
    crate::sh::parser::tokens_to_strings(expand_aliases_struct(
        crate::sh::parser::strings_to_tokens(tokens),
        aliases,
    ))
}

// -----------------------------------------------------------------------------
// Tilde expansion.
// -----------------------------------------------------------------------------
pub fn expand_tilde_struct(tokens: Vec<Token>) -> Vec<Token> {
    tokens
        .into_iter()
        .map(|t| {
            if t.is_single_quoted || t.is_double_quoted || t.is_escaped {
                t
            } else {
                Token {
                    value: expand_tilde_one(&t.value),
                    ..t
                }
            }
        })
        .collect()
}

pub fn expand_tilde(tokens: Vec<String>) -> Vec<String> {
    crate::sh::parser::tokens_to_strings(expand_tilde_struct(crate::sh::parser::strings_to_tokens(
        tokens,
    )))
}

pub fn expand_tilde_one(s: &str) -> String {
    if s.starts_with('~') {
        if s == "~" {
            return env::var("HOME").unwrap_or_else(|_| "~".into());
        }
        if s.starts_with("~/") {
            return env::var("HOME").unwrap_or_else(|_| "~".into()) + &s[1..];
        }
        if let Some((user, rest)) = s[1..].split_once('/') {
            // Validate the username to prevent path traversal via crafted
            // user names (e.g. "~../../etc/passwd").
            if user
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                && !user.is_empty()
            {
                // Try getpwnam for a correct home directory lookup.
                if let Some(home) = lookup_user_home(user) {
                    // Security fix: ensure the final resolved path does not
                    // escape the home directory by canonicalising both and
                    // checking the prefix.
                    if let Ok(canon_home) = Path::new(&home).canonicalize() {
                        let joined = Path::new(&home).join(rest);
                        if let Ok(canon_path) = joined.canonicalize() {
                            if canon_path.starts_with(&canon_home) {
                                return canon_path.to_string_lossy().into_owned();
                            }
                        }
                    }
                    // If canonicalisation fails or the path escapes, fall
                    // through to the original string without expansion.
                    return s.to_string();
                }
                // Fallback: guess /home/<user>.
                let guess = format!("/home/{}", user);
                if Path::new(&guess).is_dir() {
                    let joined = Path::new(&guess).join(rest);
                    if let (Ok(canon_home), Ok(canon_path)) =
                        (Path::new(&guess).canonicalize(), joined.canonicalize())
                    {
                        if canon_path.starts_with(&canon_home) {
                            return canon_path.to_string_lossy().into_owned();
                        }
                    }
                }
            }
        }
    }
    s.to_string()
}

/// Look up a user's home directory via getpwnam_r (thread-safe).
/// Returns None if the user does not exist or the lookup fails.
pub fn lookup_user_home(user: &str) -> Option<String> {
    let c_user = safe_cstring(user)?;
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = vec![0u8; 4096]; // 4KB buffer
    let mut pw_ptr: *mut libc::passwd = std::ptr::null_mut();
    let ret = unsafe {
        libc::getpwnam_r(
            c_user.as_ptr(),
            &mut pw,
            buf.as_mut_ptr() as *mut libc::c_char,
            buf.len(),
            &mut pw_ptr,
        )
    };
    if ret != 0 || pw_ptr.is_null() {
        return None;
    }
    let dir_ptr = pw.pw_dir;
    if dir_ptr.is_null() {
        return None;
    }
    let dir = unsafe { CStr::from_ptr(dir_ptr) }
        .to_string_lossy()
        .into_owned();
    if dir.is_empty() { None } else { Some(dir) }
}

// -----------------------------------------------------------------------------
// Variable expansion ($?, $#, $N, $$, $!, $@, $*, ${...}, $NAME).
// -----------------------------------------------------------------------------
pub fn expand_variables_struct(
    tokens: &[Token],
    state: &ShellState,
    env_cache: &HashMap<String, String>,
) -> Vec<Token> {
    tokens
        .iter()
        .map(|token| {
            if token.is_single_quoted {
                return token.clone();
            }
            let mut result = String::with_capacity(token.value.len());
            let bytes = token.value.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                // Handle backslash escaping of special characters.
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    let next = bytes[i + 1];
                    if next == b'$' || next == b'`' || next == b'"' || next == b'\\' {
                        result.push(next as char);
                        i += 2;
                        continue;
                    }
                }
                if !token.is_escaped && bytes[i] == b'$' && i + 1 < bytes.len() {
                    match bytes[i + 1] {
                        b'?' => {
                            use std::fmt::Write;
                            write!(result, "{}", state.last_status).unwrap();
                            i += 2;
                            continue;
                        }
                        b'#' => {
                            let count = state.positional_params.len();
                            result.push_str(&count.to_string());
                            i += 2;
                            continue;
                        }
                        b'$' => {
                            // Shell PID.
                            result.push_str(&std::process::id().to_string());
                            i += 2;
                            continue;
                        }
                        b'!' => {
                            // Last background PID.
                            if let Some(pid) = state.last_bg_pid {
                                result.push_str(&pid.to_string());
                            }
                            i += 2;
                            continue;
                        }
                        b'@' | b'*' => {
                            // All positional parameters joined by space.
                            result.push_str(&state.positional_params.join(" "));
                            i += 2;
                            continue;
                        }
                        b'0' => {
                            result.push_str(&state.script_name);
                            i += 2;
                            continue;
                        }
                        d if d.is_ascii_digit() => {
                            let idx = (d - b'0') as usize;
                            if idx > 0 && idx <= state.positional_params.len() {
                                result.push_str(&state.positional_params[idx - 1]);
                            }
                            i += 2;
                            continue;
                        }
                        b'{' => {
                            if let Some(end) = token.value[i + 2..].find('}') {
                                let var_name = &token.value[i + 2..i + 2 + end];
                                result.push_str(
                                    &env_cache.get(var_name).cloned().unwrap_or_default(),
                                );
                                i += 3 + end;
                                continue;
                            }
                        }
                        _ if bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_' => {
                            let start = i + 1;
                            let mut end = start;
                            while end < bytes.len()
                                && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_')
                            {
                                end += 1;
                            }
                            let var_name = std::str::from_utf8(&bytes[start..end]).unwrap();
                            result.push_str(&env_cache.get(var_name).cloned().unwrap_or_default());
                            i = end;
                            continue;
                        }
                        _ => {}
                    }
                }
                push_char_at(&token.value, &mut result, &mut i);
            }
            Token {
                value: result,
                is_single_quoted: false,
                // Preserve double quote state to prevent word splitting/globbing.
                is_double_quoted: token.is_double_quoted,
                is_escaped: false,
            }
        })
        .collect()
}

/// Legacy variable expander for string vectors.
pub fn expand_variables(
    tokens: &[String],
    state: &ShellState,
    env_cache: &HashMap<String, String>,
) -> Vec<String> {
    crate::sh::parser::tokens_to_strings(expand_variables_struct(
        &crate::sh::parser::strings_to_tokens(tokens.to_vec()),
        state,
        env_cache,
    ))
}

// -----------------------------------------------------------------------------
// Word splitting by IFS (POSIX compliant).
// -----------------------------------------------------------------------------
pub fn split_by_ifs(s: &str, ifs: &str) -> Vec<String> {
    if ifs.is_empty() {
        return vec![s.to_string()];
    }
    let mut ifs_ws = Vec::new();
    let mut ifs_non_ws = Vec::new();
    for c in ifs.chars() {
        if c.is_ascii_whitespace() {
            ifs_ws.push(c);
        } else {
            ifs_non_ws.push(c);
        }
    }
    let mut result = Vec::new();
    let mut current = String::new();
    let mut chars = s.chars().peekable();
    // Skip leading IFS whitespace.
    while let Some(&c) = chars.peek() {
        if ifs_ws.contains(&c) {
            chars.next();
        } else {
            break;
        }
    }
    while let Some(c) = chars.next() {
        if ifs_non_ws.contains(&c) {
            result.push(current.clone());
            current.clear();
            // Skip trailing IFS whitespace after non-whitespace separator.
            while let Some(&next_c) = chars.peek() {
                if ifs_ws.contains(&next_c) {
                    chars.next();
                } else {
                    break;
                }
            }
        } else if ifs_ws.contains(&c) {
            if !current.is_empty() {
                result.push(current.clone());
                current.clear();
            }
            // Skip consecutive whitespace.
            while let Some(&next_c) = chars.peek() {
                if ifs_ws.contains(&next_c) {
                    chars.next();
                } else {
                    break;
                }
            }
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

// -----------------------------------------------------------------------------
// Command substitution expansion.
// -----------------------------------------------------------------------------
pub fn expand_command_substitution_struct(
    tokens: Vec<Token>,
    state: &mut ShellState,
    depth: usize,
    rustybox_mode: bool,
    aliases: &mut HashMap<String, String>,
    functions: &mut HashMap<String, String>,
    ifs: &str,
) -> Vec<Token> {
    let mut result = Vec::new();
    for token in tokens {
        if token.is_single_quoted || token.is_escaped {
            result.push(token);
            continue;
        }
        let expanded_string = expand_command_substitution_in_token_legacy(
            &token.value,
            state,
            depth,
            rustybox_mode,
            aliases,
            functions,
        );
        if token.is_double_quoted {
            // No word splitting.  The whole expanded string becomes one token.
            result.push(Token {
                value: expanded_string,
                is_single_quoted: false,
                is_double_quoted: token.is_double_quoted,
                is_escaped: token.is_escaped,
            });
        } else {
            // Word splitting based on IFS.
            let parts = split_by_ifs(&expanded_string, ifs);
            for part in parts {
                result.push(Token {
                    value: part,
                    is_single_quoted: false,
                    is_double_quoted: false,
                    is_escaped: false,
                });
            }
        }
    }
    result
}

/// Legacy helper for string-based expansion.
pub fn expand_command_substitution_in_token_legacy(
    token: &str,
    state: &mut ShellState,
    depth: usize,
    rustybox_mode: bool,
    aliases: &mut HashMap<String, String>,
    functions: &mut HashMap<String, String>,
) -> String {
    let mut result = String::with_capacity(token.len());
    let bytes = token.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Handle backslash escaping.
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            if next == b'$' || next == b'`' || next == b'"' || next == b'\\' {
                result.push(next as char);
                i += 2;
                continue;
            }
        }
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'(' {
            // Quote-aware parenthesis matching.
            if let Some(end) = find_closing_paren(token, i + 2) {
                let cmd = &token[i + 2..end];
                let output = capture_command_output(
                    cmd,
                    state,
                    depth + 1,
                    rustybox_mode,
                    aliases,
                    functions,
                );
                result.push_str(output.trim_end_matches('\n'));
                i = end + 1; // skip past ')'
                continue;
            }
        } else if bytes[i] == b'`' {
            let mut j = i + 1;
            // Respect backslash escapes inside backticks.
            while j < bytes.len() {
                if bytes[j] == b'\\' && j + 1 < bytes.len() {
                    j += 2;
                    continue;
                }
                if bytes[j] == b'`' {
                    break;
                }
                j += 1;
            }
            if j < bytes.len() {
                let cmd = &token[i + 1..j];
                let output = capture_command_output(
                    cmd,
                    state,
                    depth + 1,
                    rustybox_mode,
                    aliases,
                    functions,
                );
                result.push_str(output.trim_end_matches('\n'));
                i = j + 1;
                continue;
            }
        }
        push_char_at(token, &mut result, &mut i);
    }
    result
}

/// Capture output of a command substitution with size and timeout limits.
/// If the output exceeds MAX_SUBSTITUTION_BYTES, the child is killed and
/// the function returns what has been read so far (truncated).
pub fn capture_command_output(
    cmd: &str,
    state: &mut ShellState,
    depth: usize,
    rustybox_mode: bool,
    aliases: &mut HashMap<String, String>,
    functions: &mut HashMap<String, String>,
) -> String {
    // Enforce depth limit to prevent stack overflow from deeply nested
    // command substitutions: $(echo $(echo $(echo ...))).
    if depth > MAX_FUNCTION_DEPTH {
        eprintln!("sh: maximum substitution nesting depth exceeded");
        return String::new();
    }
    let fds = match crate::sh::exec::pipe_cloexec() {
        Ok(fds) => fds,
        Err(_) => return String::new(),
    };
    if !acquire_child_slot() {
        unsafe {
            libc::close(fds.0);
            libc::close(fds.1);
        }
        eprintln!("sh: too many child processes");
        return String::new();
    }
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        unsafe {
            libc::close(fds.0);
            libc::close(fds.1);
        }
        release_child_slot();
        return String::new();
    }
    if pid == 0 {
        unsafe {
            // FIX 1: Create a new process group.  This allows the parent to
            // kill the entire process tree (including grandchildren spawned
            // by '&') if the command substitution times out or exceeds size
            // limits.
            libc::setpgid(0, 0);
            libc::close(fds.0);
            libc::dup2(fds.1, libc::STDOUT_FILENO);
            libc::close(fds.1); // Close original write end after dup2
            // FIX 2: Reset signal dispositions to default.
            // Crucially, SIGPIPE must be SIG_DFL to prevent CPU exhaustion
            // in pipelines (e.g., if a grandchild ignores it).
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::signal(libc::SIGQUIT, libc::SIG_DFL);
            libc::signal(libc::SIGPIPE, libc::SIG_DFL);
            // Reset per-process child counter.
            ACTIVE_CHILDREN.store(0, Ordering::SeqCst);
            let status = run_single_command_list(
                cmd,
                state,
                depth,
                rustybox_mode,
                aliases,
                functions,
                false,
            );
            libc::_exit(status as i32);
        }
    }
    // Parent process.
    unsafe {
        libc::close(fds.1); // Parent only reads
        // Race-free process group setup (both parent and child call setpgid).
        libc::setpgid(pid, pid);
    }
    // Use poll() for timeout instead of setsockopt(SO_RCVTIMEO) which only
    // works on sockets.
    let mut pfd = libc::pollfd {
        fd: fds.0,
        events: libc::POLLIN,
        revents: 0,
    };
    let timeout_ms = (SUBSTITUTION_READ_TIMEOUT_USEC / 1000) as i32;
    // Read output with a size limit to prevent memory exhaustion.
    let mut output = String::new();
    let mut buf = [0u8; 4096];
    let mut total_read: usize = 0;
    loop {
        // Check for user interrupt to allow breaking out of long reads.
        if SIGINT_RECEIVED.load(Ordering::SeqCst) {
            unsafe {
                // FIX 3: Kill the entire process group, not just the direct
                // child.
                libc::kill(-pid, libc::SIGINT);
            }
            break;
        }
        let ret = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
        if ret < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        if ret == 0 {
            // Timeout expired.
            eprintln!("sh: command substitution timed out");
            unsafe {
                // FIX 3: Kill the entire process group to ensure
                // grandchildren release their inherited copies of the pipe
                // write end.
                libc::kill(-pid, libc::SIGKILL);
            }
            break;
        }
        let n = unsafe { libc::read(fds.0, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        if n == 0 {
            break; // EOF reached
        }
        let n = n as usize;
        total_read += n;
        if total_read > MAX_SUBSTITUTION_BYTES {
            eprintln!(
                "sh: command substitution output truncated (limit {} bytes)",
                MAX_SUBSTITUTION_BYTES
            );
            unsafe {
                // FIX 3: Kill the entire process group.
                libc::kill(-pid, libc::SIGKILL);
            }
            // Drain any already-pending data to prevent SIGPIPE in the child.
            // Since we killed the process group, the pipe will quickly reach
            // EOF (POLLHUP).
            let drain_timeout = 100; // 100ms
            let mut pfd_drain = libc::pollfd {
                fd: fds.0,
                events: libc::POLLIN,
                revents: 0,
            };
            loop {
                let ret_drain = unsafe { libc::poll(&mut pfd_drain, 1, drain_timeout) };
                if ret_drain <= 0 {
                    break; // no more data, error, or timeout
                }
                // FIX 4: Check for hangup to exit drain loop immediately.
                if (pfd_drain.revents & libc::POLLHUP) != 0 {
                    break;
                }
                let d =
                    unsafe { libc::read(fds.0, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
                if d <= 0 {
                    break;
                }
            }
            break;
        }
        output.push_str(&String::from_utf8_lossy(&buf[..n]));
    }
    unsafe {
        libc::close(fds.0);
    }
    let mut status: i32 = 0;
    unsafe {
        // Wait for the direct child to prevent zombies.
        // Grandchildren will be orphaned and adopted by init, which is fine
        // since they have been killed and no longer hold the pipe open.
        libc::waitpid(pid, &mut status, 0);
    }
    release_child_slot();
    output
}

// -----------------------------------------------------------------------------
// Globbing.
// -----------------------------------------------------------------------------
pub fn expand_globs_struct(tokens: Vec<Token>) -> Vec<Token> {
    let mut result = Vec::new();
    for token in tokens {
        // Do not expand globs if the token was single-quoted, double-quoted,
        // or escaped.
        if token.is_single_quoted || token.is_escaped || token.is_double_quoted {
            result.push(token);
            continue;
        }
        if token.value.contains('*') || token.value.contains('?') || token.value.contains('[') {
            match glob(&token.value) {
                Ok(matches) if !matches.is_empty() => {
                    for m in matches {
                        result.push(Token {
                            value: m,
                            is_single_quoted: false,
                            is_double_quoted: false,
                            is_escaped: false,
                        });
                    }
                }
                _ => result.push(token),
            }
        } else {
            result.push(token);
        }
    }
    result
}

pub fn expand_globs(tokens: Vec<String>) -> Vec<String> {
    crate::sh::parser::tokens_to_strings(expand_globs_struct(crate::sh::parser::strings_to_tokens(
        tokens,
    )))
}

pub fn glob(pattern: &str) -> io::Result<Vec<String>> {
    let path = Path::new(pattern);
    let (dir, file_pattern) = if pattern.contains('/') {
        (
            path.parent().unwrap_or(Path::new(".")).to_path_buf(),
            path.file_name()
                .unwrap_or_default()
                .to_str()
                .unwrap_or("")
                .to_string(),
        )
    } else {
        (Path::new(".").to_path_buf(), pattern.to_string())
    };
    let mut matches = Vec::new();
    if dir.is_dir() {
        let mut dir_iterations = 0;
        for entry in std::fs::read_dir(&dir)? {
            dir_iterations += 1;
            if dir_iterations > 100_000 {
                // Strict scanning limit.
                eprintln!("sh: glob: directory too large, truncating results");
                break;
            }
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if matches_pattern(&name, &file_pattern) {
                matches.push(dir.join(&name).to_string_lossy().into_owned());
                if matches.len() >= MAX_GLOB_RESULTS {
                    break;
                }
            }
        }
    }
    matches.sort();
    Ok(matches)
}

pub fn matches_pattern(name: &str, pattern: &str) -> bool {
    let name_bytes = name.as_bytes();
    let pat_bytes = pattern.as_bytes();
    let mut ni = 0;
    let mut pi = 0;
    let mut star = None;
    let mut match_start = 0;
    let mut iterations: usize = 0;
    while ni < name_bytes.len() {
        // ReDoS mitigation: abort on pathological patterns.
        iterations += 1;
        if iterations > MAX_GLOB_ITERATIONS {
            return false;
        }
        if pi < pat_bytes.len() && pat_bytes[pi] == b'*' {
            star = Some(pi);
            match_start = ni;
            pi += 1;
        } else if pi < pat_bytes.len() && (pat_bytes[pi] == b'?' || pat_bytes[pi] == name_bytes[ni])
        {
            ni += 1;
            pi += 1;
        } else if pi < pat_bytes.len() && pat_bytes[pi] == b'[' {
            pi += 1;
            let mut matched = false;
            let mut negate = false;
            if pi < pat_bytes.len() && pat_bytes[pi] == b'!' {
                negate = true;
                pi += 1;
            }
            // Handle ']' as the first character in the bracket (literal).
            if pi < pat_bytes.len() && pat_bytes[pi] == b']' {
                if ni < name_bytes.len() && name_bytes[ni] == b']' {
                    matched = true;
                }
                pi += 1;
            }
            while pi < pat_bytes.len() && pat_bytes[pi] != b']' {
                if pi + 2 < pat_bytes.len() && pat_bytes[pi + 1] == b'-' {
                    let start = pat_bytes[pi];
                    let end = pat_bytes[pi + 2];
                    if ni < name_bytes.len() && name_bytes[ni] >= start && name_bytes[ni] <= end {
                        matched = true;
                    }
                    pi += 3;
                } else {
                    if ni < name_bytes.len() && name_bytes[ni] == pat_bytes[pi] {
                        matched = true;
                    }
                    pi += 1;
                }
            }
            // If we reached end of pattern without finding ']', treat the
            // bracket expression as a literal non-match (fail closed).
            if pi >= pat_bytes.len() {
                return false;
            }
            // Skip closing ']'.
            pi += 1;
            if matched == negate {
                if let Some(s) = star {
                    pi = s + 1;
                    match_start += 1;
                    ni = match_start;
                } else {
                    return false;
                }
            } else {
                ni += 1;
            }
        } else if let Some(s) = star {
            pi = s + 1;
            match_start += 1;
            ni = match_start;
        } else {
            return false;
        }
    }
    while pi < pat_bytes.len() && pat_bytes[pi] == b'*' {
        pi += 1;
    }
    pi == pat_bytes.len()
}
