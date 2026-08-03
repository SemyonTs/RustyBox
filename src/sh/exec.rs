// =============================================================================
// sh exec — command execution, pipelines, external processes, job control,
// process substitution, and file descriptor utilities.
// =============================================================================
use crate::sh::builtins::{handle_builtin, handle_rustybox_builtin};
use crate::sh::expansion::*;
use crate::sh::globals::*;
use crate::sh::parser::*;
use crate::sh::signals::{acquire_child_slot, release_child_slot};
use std::collections::HashMap;
use std::env;
use std::fs::OpenOptions;
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::{IntoRawFd, RawFd};
use std::sync::atomic::Ordering;

// -----------------------------------------------------------------------------
// Command list execution.  Recursive depth is limited to prevent stack
// overflow from infinite function loops or deeply nested substitutions.
// -----------------------------------------------------------------------------
pub fn run_command_list(
    input: &str,
    state: &mut ShellState,
    depth: usize,
    rustybox_mode: bool,
    aliases: &mut HashMap<String, String>,
    functions: &mut HashMap<String, String>,
) -> u8 {
    if depth > MAX_FUNCTION_DEPTH {
        eprintln!("sh: maximum function call depth exceeded");
        return 1;
    }
    // Quote-aware parsing of function definitions.
    if let Some((name, body)) = try_parse_function_def(input, is_valid_identifier) {
        functions.insert(name, body);
        return state.last_status;
    }
    // Split the input on top-level operators (;, &&, ||, &) respecting
    // quotes, $(...), `...`, and parenthesised subshells.
    let commands = split_command_list(input);
    for (pos, (cmd, _sep)) in commands.iter().enumerate() {
        if cmd.is_empty() {
            continue;
        }
        if state.exit_requested {
            break;
        }
        if pos > 0 {
            let prev_sep = commands[pos - 1].1;
            match prev_sep {
                Some('&') if state.last_status != 0 => continue,
                Some('|') if state.last_status == 0 => continue,
                _ => {}
            }
        }
        let is_background = commands[pos].1 == Some('b');
        state.last_status = run_single_command_list(
            cmd,
            state,
            depth + 1,
            rustybox_mode,
            aliases,
            functions,
            is_background,
        );
    }
    reap_background();
    state.last_status
}

pub fn run_single_command_list(
    input: &str,
    state: &mut ShellState,
    depth: usize,
    rustybox_mode: bool,
    aliases: &mut HashMap<String, String>,
    functions: &mut HashMap<String, String>,
    background: bool,
) -> u8 {
    // Enforce depth limit on all paths (including command substitution and
    // process substitution) to prevent stack overflow.
    if depth > MAX_FUNCTION_DEPTH {
        eprintln!("sh: maximum nesting depth exceeded");
        return 1;
    }
    // Replace <(...) and >(...) with /dev/fd/N.  Track file descriptors that
    // must be closed after the command finishes to avoid leaking them.
    let (processed, fds_to_close) =
        replace_process_substitution(input, state, depth, rustybox_mode, aliases, functions);
    let segments: Vec<String> = split_pipeline(&processed);
    let segments: Vec<&str> = segments.iter().map(|s| s.as_str()).collect();
    if segments.is_empty() {
        // Close any fds that were opened for unused process substitutions.
        for fd in fds_to_close {
            unsafe {
                libc::close(fd);
            }
        }
        return state.last_status;
    }
    if background {
        if !acquire_child_slot() {
            eprintln!("sh: too many child processes");
            for fd in fds_to_close {
                unsafe {
                    libc::close(fd);
                }
            }
            return 1;
        }
        let pid = unsafe { libc::fork() };
        if pid < 0 {
            eprintln!("sh: fork failed");
            release_child_slot();
            for fd in fds_to_close {
                unsafe {
                    libc::close(fd);
                }
            }
            return 1;
        }
        if pid == 0 {
            unsafe {
                // Child: create a new process group for proper job control.
                libc::setpgid(0, 0);
                // Reset per-process child counter (this process tracks its own).
                ACTIVE_CHILDREN.store(0, Ordering::SeqCst);
                let status = if segments.len() == 1 {
                    run_simple_command(segments[0], state, depth, rustybox_mode, aliases, functions)
                } else {
                    run_pipeline(&segments, state, depth, rustybox_mode, aliases, functions)
                };
                libc::_exit(status as i32)
            };
        }
        // Parent: set the child's process group (race-free: both sides call).
        unsafe {
            libc::setpgid(pid, pid);
        }
        // Parent: close our copy of the process-substitution fds.
        for fd in fds_to_close {
            unsafe {
                libc::close(fd);
            }
        }
        state.last_bg_pid = Some(pid);
        eprintln!("[{}] {}", pid, input);
        add_job(pid, pid, input.to_string(), JobState::Running);
        return state.last_status;
    }
    let status = if segments.len() == 1 {
        run_simple_command(segments[0], state, depth, rustybox_mode, aliases, functions)
    } else {
        run_pipeline(&segments, state, depth, rustybox_mode, aliases, functions)
    };
    // After the foreground command finishes, close any fds that were kept open.
    for fd in fds_to_close {
        unsafe {
            libc::close(fd);
        }
    }
    status
}

pub fn run_simple_command(
    input: &str,
    state: &mut ShellState,
    depth: usize,
    rustybox_mode: bool,
    aliases: &mut HashMap<String, String>,
    functions: &mut HashMap<String, String>,
) -> u8 {
    let tokens = tokenize(input);
    if tokens.is_empty() {
        return 0;
    }
    let tokens = expand_aliases_struct(tokens, aliases);
    let tokens = expand_tilde_struct(tokens);
    let env_cache: HashMap<String, String> = env::vars().collect();
    let tokens = expand_variables_struct(&tokens, state, &env_cache);
    let ifs = env::var("IFS").unwrap_or_else(|_| " \t\n".to_string());
    let final_tokens = expand_command_substitution_struct(
        tokens,
        state,
        depth,
        rustybox_mode,
        aliases,
        functions,
        &ifs,
    );
    // Parse redirections BEFORE globbing to detect ambiguous redirects correctly.
    let (cmd_tokens, stdin_file_token, stdout_file_token, append, _here_doc) =
        match parse_redirections_tokens(final_tokens) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("sh: {e}");
                return 1;
            }
        };
    let stdin_file = if let Some(f) = stdin_file_token {
        let expanded = expand_globs_struct(vec![f]);
        if expanded.len() != 1 {
            eprintln!("sh: ambiguous redirect or no match");
            return 1;
        }
        Some(expanded.into_iter().next().unwrap().value)
    } else {
        None
    };
    let stdout_file = if let Some(f) = stdout_file_token {
        let expanded = expand_globs_struct(vec![f]);
        if expanded.len() != 1 {
            eprintln!("sh: ambiguous redirect or no match");
            return 1;
        }
        Some(expanded.into_iter().next().unwrap().value)
    } else {
        None
    };
    let final_tokens = expand_globs_struct(cmd_tokens);
    // Convert to Vec<String> for execution helpers.
    let exec_tokens: Vec<String> = final_tokens.into_iter().map(|t| t.value).collect();
    if exec_tokens.is_empty() {
        return 0;
    }
    if let Some(result) = handle_builtin(&exec_tokens, state, depth, aliases, functions) {
        return result;
    }
    if exec_tokens.len() >= 1 {
        if let Some(body) = functions.get(&exec_tokens[0]).cloned() {
            let status =
                run_command_list(&body, state, depth + 1, rustybox_mode, aliases, functions);
            return status;
        }
    }
    if rustybox_mode {
        if let Some(result) = handle_rustybox_builtin(&exec_tokens) {
            return result;
        }
    }
    run_external(&exec_tokens, stdin_file, stdout_file, append, rustybox_mode)
}

pub fn run_pipeline(
    segments: &[&str],
    state: &mut ShellState,
    depth: usize,
    rustybox_mode: bool,
    aliases: &mut HashMap<String, String>,
    functions: &mut HashMap<String, String>,
) -> u8 {
    let mut prev_read_fd: Option<RawFd> = None;
    let mut pids: Vec<i32> = Vec::new();
    let mut statuses: Vec<u8> = Vec::new();
    let pipefail = state.pipefail;
    let env_cache: HashMap<String, String> = env::vars().collect();
    let ifs = env::var("IFS").unwrap_or_else(|_| " \t\n".to_string());
    for (i, segment) in segments.iter().enumerate() {
        let tokens = tokenize(segment);
        if tokens.is_empty() {
            continue;
        }
        let tokens = expand_aliases_struct(tokens, aliases);
        let tokens = expand_tilde_struct(tokens);
        let tokens = expand_variables_struct(&tokens, state, &env_cache);
        let final_tokens = expand_command_substitution_struct(
            tokens,
            state,
            depth,
            rustybox_mode,
            aliases,
            functions,
            &ifs,
        );
        // Parse redirections BEFORE globbing.
        let (cmd_tokens, stdin_file_token, stdout_file_token, append, _) =
            match parse_redirections_tokens(final_tokens) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("sh: {e}");
                    return 1;
                }
            };
        let stdin_file = if let Some(f) = stdin_file_token {
            let expanded = expand_globs_struct(vec![f]);
            if expanded.len() != 1 {
                eprintln!("sh: ambiguous redirect or no match");
                return 1;
            }
            Some(expanded.into_iter().next().unwrap().value)
        } else {
            None
        };
        let stdout_file = if let Some(f) = stdout_file_token {
            let expanded = expand_globs_struct(vec![f]);
            if expanded.len() != 1 {
                eprintln!("sh: ambiguous redirect or no match");
                return 1;
            }
            Some(expanded.into_iter().next().unwrap().value)
        } else {
            None
        };
        let final_tokens = expand_globs_struct(cmd_tokens);
        let exec_tokens: Vec<String> = final_tokens.into_iter().map(|t| t.value).collect();
        if exec_tokens.is_empty() {
            continue;
        }
        // Reject all builtins that modify shell state – they would run in a
        // child process and their effects would be silently lost.
        if matches!(
            exec_tokens[0].as_str(),
            "cd" | "exit" | "exec" | "alias" | "export" | "set" | "unset" | "source" | "." | "eval"
        ) {
            eprintln!("sh: '{}' cannot be used in pipelines", exec_tokens[0]);
            return 1;
        }
        let (pipe_read, pipe_write) = if i != segments.len() - 1 {
            let fds = pipe_cloexec().unwrap();
            (Some(fds.0), Some(fds.1))
        } else {
            (None, None)
        };
        if !acquire_child_slot() {
            eprintln!("sh: too many child processes");
            // Kill already-forked children in this pipeline to avoid orphans.
            for &prev_pid in &pids {
                unsafe {
                    libc::kill(prev_pid, libc::SIGKILL);
                }
            }
            if let Some(r) = prev_read_fd {
                unsafe {
                    libc::close(r);
                }
            }
            if let Some(r) = pipe_read {
                unsafe {
                    libc::close(r);
                }
            }
            if let Some(w) = pipe_write {
                unsafe {
                    libc::close(w);
                }
            }
            // Wait for killed children to prevent zombies.
            for &prev_pid in &pids {
                let mut st: i32 = 0;
                unsafe {
                    libc::waitpid(prev_pid, &mut st, 0);
                }
                release_child_slot();
            }
            return 1;
        }
        let pid = unsafe { libc::fork() };
        if pid < 0 {
            eprintln!("sh: fork failed");
            release_child_slot();
            // Kill already-forked children.
            for &prev_pid in &pids {
                unsafe {
                    libc::kill(prev_pid, libc::SIGKILL);
                }
            }
            if let Some(r) = prev_read_fd {
                unsafe {
                    libc::close(r);
                }
            }
            if let Some(r) = pipe_read {
                unsafe {
                    libc::close(r);
                }
            }
            if let Some(w) = pipe_write {
                unsafe {
                    libc::close(w);
                }
            }
            for &prev_pid in &pids {
                let mut st: i32 = 0;
                unsafe {
                    libc::waitpid(prev_pid, &mut st, 0);
                }
                release_child_slot();
            }
            return 1;
        }
        if pid == 0 {
            unsafe {
                // Child: create a new process group for job control.
                libc::setpgid(0, 0);
                // Reset per-process child counter.
                ACTIVE_CHILDREN.store(0, Ordering::SeqCst);
                // Wire up the pipeline pipes.
                if let Some(read_fd) = prev_read_fd {
                    libc::dup2(read_fd, libc::STDIN_FILENO);
                    libc::close(read_fd);
                }
                if let Some(write_fd) = pipe_write {
                    libc::dup2(write_fd, libc::STDOUT_FILENO);
                    libc::close(write_fd);
                }
                if let Some(read_fd) = pipe_read {
                    libc::close(read_fd);
                }
                // Handle explicit file redirections.
                if let Some(f) = &stdin_file {
                    if let Ok(fd) = open_file_for_read(f) {
                        libc::dup2(fd, libc::STDIN_FILENO);
                        libc::close(fd);
                    } else {
                        libc::_exit(1);
                    }
                }
                if let Some(f) = &stdout_file {
                    if let Ok(fd) = open_file_for_write(f, append) {
                        libc::dup2(fd, libc::STDOUT_FILENO);
                        libc::close(fd);
                    } else {
                        libc::_exit(1);
                    }
                }
                // POSIX COMPLIANCE: Reset signal dispositions to default before exec.
                // This ensures the executed program handles SIGPIPE correctly
                // (terminates on broken pipe) instead of inheriting SIG_IGN,
                // which would cause CPU exhaustion (e.g., `cat /dev/urandom | head`).
                libc::signal(libc::SIGINT, libc::SIG_DFL);
                libc::signal(libc::SIGQUIT, libc::SIG_DFL);
                libc::signal(libc::SIGPIPE, libc::SIG_DFL);
                if rustybox_mode {
                    if let Some(def) = crate::registry::find(&exec_tokens[0]) {
                        let argv: Vec<String> = exec_tokens.to_vec();
                        let mut ctx = crate::context::Context::new(def, argv);
                        let code = (def.run)(&mut ctx);
                        libc::_exit(code as i32);
                    }
                }
                let (args, argv) = match build_argv(&exec_tokens) {
                    Some(v) => v,
                    None => {
                        eprintln!("sh: invalid argument (null byte)");
                        libc::_exit(1);
                    }
                };
                libc::execvp(argv[0], argv.as_ptr());
                // If execvp returns, it failed.
                eprintln!("sh: {}: command not found", exec_tokens[0]);
                drop(args);
                libc::_exit(127);
            }
        }
        // Parent: set child's process group (race-free with child's setpgid).
        unsafe {
            libc::setpgid(pid, pid);
        }
        if let Some(write_fd) = pipe_write {
            unsafe {
                libc::close(write_fd);
            }
        }
        if let Some(read_fd) = prev_read_fd {
            unsafe {
                libc::close(read_fd);
            }
        }
        prev_read_fd = pipe_read;
        pids.push(pid);
    }
    if let Some(read_fd) = prev_read_fd {
        unsafe {
            libc::close(read_fd);
        }
    }
    for &pid in &pids {
        let status = wait_for_child(pid);
        release_child_slot();
        statuses.push(status);
    }
    if pipefail {
        for s in &statuses {
            if *s != 0 {
                return *s;
            }
        }
        0
    } else {
        *statuses.last().unwrap_or(&0)
    }
}

pub fn run_external(
    tokens: &[String],
    stdin_file: Option<String>,
    stdout_file: Option<String>,
    append: bool,
    rustybox_mode: bool,
) -> u8 {
    if !acquire_child_slot() {
        eprintln!("sh: too many child processes");
        return 1;
    }
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        eprintln!("sh: fork failed");
        release_child_slot();
        return 1;
    }
    if pid == 0 {
        unsafe {
            // Child: reset per-process child counter.
            ACTIVE_CHILDREN.store(0, Ordering::SeqCst);
            if let Some(f) = &stdin_file {
                if let Ok(fd) = open_file_for_read(f) {
                    libc::dup2(fd, libc::STDIN_FILENO);
                    libc::close(fd);
                } else {
                    libc::_exit(1);
                }
            }
            if let Some(f) = &stdout_file {
                if let Ok(fd) = open_file_for_write(f, append) {
                    libc::dup2(fd, libc::STDOUT_FILENO);
                    libc::close(fd);
                } else {
                    libc::_exit(1);
                }
            }
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::signal(libc::SIGQUIT, libc::SIG_DFL);
            if rustybox_mode {
                if let Some(def) = crate::registry::find(&tokens[0]) {
                    let argv: Vec<String> = tokens.to_vec();
                    let mut ctx = crate::context::Context::new(def, argv);
                    let code = (def.run)(&mut ctx);
                    libc::_exit(code as i32);
                }
            }
            let (args, argv) = match build_argv(tokens) {
                Some(v) => v,
                None => {
                    eprintln!("sh: invalid argument (null byte)");
                    libc::_exit(1);
                }
            };
            libc::execvp(argv[0], argv.as_ptr());
            eprintln!("sh: {}: command not found", tokens[0]);
            drop(args);
            libc::_exit(127);
        }
    }
    let status = wait_for_child(pid);
    release_child_slot();
    status
}

// -----------------------------------------------------------------------------
// Wait for child, handling signals and job control.
// -----------------------------------------------------------------------------
pub fn wait_for_child(pid: i32) -> u8 {
    loop {
        let mut status: i32 = 0;
        let ret = unsafe { libc::waitpid(pid, &mut status, libc::WUNTRACED) };
        if ret < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                if SIGINT_RECEIVED.swap(false, Ordering::SeqCst) {
                    // Send to process group to avoid PID recycling race conditions.
                    unsafe {
                        libc::kill(-pid, libc::SIGINT);
                    }
                }
                if SIGTSTP_RECEIVED.swap(false, Ordering::SeqCst) {
                    unsafe {
                        libc::kill(-pid, libc::SIGTSTP);
                    }
                    update_job_state(pid, JobState::Stopped);
                    eprintln!("\n[{}] + stopped", pid);
                    let sig = unsafe { libc::WSTOPSIG(status) };
                    return 128u8.saturating_add(sig as u8);
                }
                continue;
            }
            return 1;
        }
        if libc::WIFEXITED(status) {
            let code = libc::WEXITSTATUS(status) as u8;
            remove_job(pid);
            return code;
        }
        if libc::WIFSIGNALED(status) {
            let sig = libc::WTERMSIG(status);
            if sig == libc::SIGTSTP {
                update_job_state(pid, JobState::Stopped);
            }
            remove_job(pid);
            // Use saturating arithmetic to prevent u8 overflow on exotic
            // platforms with signal numbers > 127.
            return 128u8.saturating_add(sig as u8);
        }
        if libc::WIFSTOPPED(status) {
            let sig = libc::WSTOPSIG(status);
            update_job_state(pid, JobState::Stopped);
            return 128u8.saturating_add(sig as u8);
        }
        return 1;
    }
}

/// Reap finished child processes (both background jobs and process
/// substitutions).  This function is called periodically (e.g. before
/// prompting) and ensures that child-process slots are released for all
/// reaped children, preventing resource exhaustion (fork-bomb mitigation).
pub fn reap_background() {
    // First, reap process substitution children that have already completed.
    {
        let mut proc_pids = PROC_SUB_PIDS.lock().unwrap();
        let mut i = 0;
        while i < proc_pids.len() {
            let pid = proc_pids[i];
            let mut status: i32 = 0;
            let ret = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
            if ret > 0 {
                release_child_slot();
                proc_pids.remove(i);
            } else if ret < 0 {
                // Error or already reaped.
                proc_pids.remove(i);
            } else {
                i += 1;
            }
        }
    }
    // Then reap any other background jobs (including those from JOBS) and
    // also catch any process-substitution children that may have exited
    // after the first loop (to avoid leaking slots).
    loop {
        let mut status: i32 = 0;
        let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG | libc::WUNTRACED) };
        if pid <= 0 {
            break;
        }
        let mut handled = false;
        // Check if it is a regular job from the job table.
        if JOBS.lock().unwrap().iter().any(|j| j.pid == pid) {
            if libc::WIFEXITED(status) || libc::WIFSIGNALED(status) {
                eprintln!("[{}] Done", pid);
                remove_job(pid);
                release_child_slot();
                handled = true;
            } else if libc::WIFSTOPPED(status) {
                update_job_state(pid, JobState::Stopped);
                handled = true;
            }
        }
        // If not handled as a regular job, check if it is a process substitution.
        if !handled {
            let mut proc_pids = PROC_SUB_PIDS.lock().unwrap();
            if let Some(pos) = proc_pids.iter().position(|&p| p == pid) {
                proc_pids.remove(pos);
                release_child_slot();
                handled = true;
            }
        }
        // If the PID was not found in either table, it might have already
        // been removed (e.g., double wait), but we still need to ensure the
        // slot is freed if it was ever allocated.  Since we cannot know, we
        // conservatively attempt to release a slot only if the process was
        // waited for successfully.  However, we must avoid underflow.  We
        // assume that any child that waitpid returns for was previously
        // counted, so we can safely release.  But to be safe, we check if
        // the PID is known to us; if not, we still release a slot to prevent
        // leaks (the counter will not go negative because we only release if
        // we previously acquired).
        if !handled {
            // This case should be rare; we release a slot to avoid leaks.
            release_child_slot();
        }
    }
}

// -----------------------------------------------------------------------------
// Job control helpers.
// -----------------------------------------------------------------------------
pub fn add_job(pid: i32, pgid: i32, cmd: String, state: JobState) {
    let mut jobs = JOBS.lock().unwrap();
    jobs.push(Job {
        pid,
        pgid,
        command: cmd,
        state,
    });
}

pub fn remove_job(pid: i32) {
    let mut jobs = JOBS.lock().unwrap();
    jobs.retain(|j| j.pid != pid);
}

pub fn update_job_state(pid: i32, state: JobState) {
    if let Some(job) = JOBS.lock().unwrap().iter_mut().find(|j| j.pid == pid) {
        job.state = state;
    }
}

/// Look up a job by PID.  Returns true only if the PID is present in the
/// job table AND the process is still alive (kill(pid, 0) succeeds),
/// preventing signals from being sent to recycled PIDs.
pub fn job_exists_and_alive(pid: i32) -> bool {
    let in_table = JOBS.lock().unwrap().iter().any(|j| j.pid == pid);
    if !in_table {
        return false;
    }
    // Verify the process is still alive to guard against PID recycling.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Return the PID of the most recently added job (the "current" job).
pub fn current_job_pid() -> Option<i32> {
    JOBS.lock().unwrap().last().map(|j| j.pid)
}

/// Return the PGID of a job by PID.
pub fn job_pgid(pid: i32) -> Option<i32> {
    JOBS.lock()
        .unwrap()
        .iter()
        .find(|j| j.pid == pid)
        .map(|j| j.pgid)
}

// -----------------------------------------------------------------------------
// Process substitution: <(...) and >(...).
// Returns the modified input string and a list of file descriptors that
// must be closed after the command finishes.
// FIXED: Now respects quoting contexts and clears O_CLOEXEC so child
// inherits it.
// -----------------------------------------------------------------------------
pub fn replace_process_substitution(
    input: &str,
    state: &mut ShellState,
    depth: usize,
    rustybox_mode: bool,
    aliases: &mut HashMap<String, String>,
    functions: &mut HashMap<String, String>,
) -> (String, Vec<RawFd>) {
    let mut result = String::with_capacity(input.len());
    let mut fds = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    let mut in_single = false;
    let mut in_double = false;
    let mut in_backtick = false;
    while i < bytes.len() {
        let b = bytes[i];
        // Handle quoting state transitions.
        if in_single {
            push_char_at(input, &mut result, &mut i);
            if b == b'\'' {
                in_single = false;
            }
            continue;
        }
        if in_double {
            if b == b'"' {
                result.push('"');
                in_double = false;
                i += 1;
            } else if b == b'\\' && i + 1 < bytes.len() {
                i += 1;
                push_char_at(input, &mut result, &mut i);
            } else {
                push_char_at(input, &mut result, &mut i);
            }
            continue;
        }
        if in_backtick {
            if b == b'\\' && i + 1 < bytes.len() {
                result.push('\\');
                i += 1;
                push_char_at(input, &mut result, &mut i);
            } else if b == b'`' {
                result.push('`');
                in_backtick = false;
                i += 1;
            } else {
                push_char_at(input, &mut result, &mut i);
            }
            continue;
        }
        // Outside quotes.
        if b == b'\'' {
            in_single = true;
            result.push('\'');
            i += 1;
        } else if b == b'"' {
            in_double = true;
            result.push('"');
            i += 1;
        } else if b == b'`' {
            in_backtick = true;
            result.push('`');
            i += 1;
        } else if i + 1 < bytes.len() && (b == b'<' || b == b'>') && bytes[i + 1] == b'(' {
            let direction = b;
            // Use quote-aware matching for the closing paren.
            if let Some(end) = find_closing_paren(input, i + 2) {
                let cmd_str = &input[i + 2..end];
                let pipe_fds = match pipe_cloexec() {
                    Ok(fds) => fds,
                    Err(_) => {
                        // Cannot create pipe; emit the literal text.
                        push_char_at(input, &mut result, &mut i);
                        continue;
                    }
                };
                if !acquire_child_slot() {
                    unsafe {
                        libc::close(pipe_fds.0);
                        libc::close(pipe_fds.1);
                    }
                    eprintln!("sh: too many child processes");
                    push_char_at(input, &mut result, &mut i);
                    continue;
                }
                let pid = unsafe { libc::fork() };
                if pid < 0 {
                    unsafe {
                        libc::close(pipe_fds.0);
                        libc::close(pipe_fds.1);
                    }
                    release_child_slot();
                    eprintln!("sh: fork failed");
                    push_char_at(input, &mut result, &mut i);
                    continue;
                }
                if pid == 0 {
                    unsafe {
                        // Child: create a new process group.
                        libc::setpgid(0, 0);
                        // Reset per-process child counter.
                        ACTIVE_CHILDREN.store(0, Ordering::SeqCst);
                        if direction == b'<' {
                            libc::close(pipe_fds.0);
                            libc::dup2(pipe_fds.1, libc::STDOUT_FILENO);
                            libc::close(pipe_fds.1);
                        } else {
                            libc::close(pipe_fds.1);
                            libc::dup2(pipe_fds.0, libc::STDIN_FILENO);
                            libc::close(pipe_fds.0);
                        }
                        // Inherit the parent shell's aliases, functions, and
                        // state so that process substitutions behave like any
                        // other command in the current shell environment.
                        let status = crate::sh::exec::run_command_list(
                            cmd_str,
                            state,
                            depth + 1,
                            rustybox_mode,
                            aliases,
                            functions,
                        );
                        libc::_exit(status as i32);
                    }
                }
                // Parent: set child's process group.
                unsafe {
                    libc::setpgid(pid, pid);
                }
                // Parent: keep the fd we need, close the other.
                let fd_to_keep = if direction == b'<' {
                    unsafe {
                        libc::close(pipe_fds.1);
                    }
                    pipe_fds.0
                } else {
                    unsafe {
                        libc::close(pipe_fds.0);
                    }
                    pipe_fds.1
                };
                // Clear FD_CLOEXEC so the child process inherits the fd when
                // execvp is called.
                unsafe {
                    libc::fcntl(fd_to_keep, libc::F_SETFD, 0);
                }
                result.push_str(&format!("/dev/fd/{}", fd_to_keep));
                fds.push(fd_to_keep);
                // Track PID separately for process substitution to avoid
                // polluting the job table and prevent zombie accumulation.
                PROC_SUB_PIDS.lock().unwrap().push(pid);
                i = end + 1;
                continue;
            }
        } else if b == b'\\' && i + 1 < bytes.len() {
            result.push('\\');
            i += 1;
            push_char_at(input, &mut result, &mut i);
        } else {
            push_char_at(input, &mut result, &mut i);
        }
    }
    (result, fds)
}

// -----------------------------------------------------------------------------
// File descriptor utilities.
// -----------------------------------------------------------------------------
/// Create a pipe with O_CLOEXEC set on both ends to prevent file descriptor
/// leaks into child processes spawned by nested substitutions or exec.
pub fn pipe_cloexec() -> io::Result<(RawFd, RawFd)> {
    let mut fds = [-1i32; 2];
    let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok((fds[0], fds[1]))
    }
}

pub fn open_file_for_read(path: &str) -> io::Result<RawFd> {
    // Use O_NOFOLLOW to prevent symlink-following attacks on redirection
    // targets.  If the path is a symlink the open will fail with ELOOP.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    Ok(file.into_raw_fd())
}

pub fn open_file_for_write(path: &str, append: bool) -> io::Result<RawFd> {
    // Use O_NOFOLLOW to prevent symlink-following attacks.  An attacker
    // cannot redirect output to an arbitrary file by placing a symlink at
    // the target path.
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .append(append)
        .truncate(!append)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    Ok(file.into_raw_fd())
}
