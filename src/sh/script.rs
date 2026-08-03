// =============================================================================
// sh script — script execution with shebang support and shell argument
// parsing for non-interactive mode.
// =============================================================================
use crate::sh::exec::run_command_list;
use crate::sh::globals::*;
use crate::sh::signals::{acquire_child_slot, release_child_slot};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{self, BufRead, BufReader, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::atomic::Ordering;

// -----------------------------------------------------------------------------
// Shell argument parsing for script mode.
// -----------------------------------------------------------------------------
pub fn parse_shell_arguments(args: &[String]) -> (Option<String>, Option<String>, Vec<String>) {
    if args.is_empty() {
        return (None, None, vec![]);
    }
    if args[0] == "-c" {
        if args.len() < 2 {
            eprintln!("sh: -c requires an argument");
            return (Some(String::new()), None, vec![]);
        }
        let cmd = args[1].clone();
        let rest = if args.len() > 2 {
            args[2..].to_vec()
        } else {
            vec![]
        };
        return (Some(cmd), None, rest);
    }
    let script = args[0].clone();
    let script_args = if args.len() > 1 {
        args[1..].to_vec()
    } else {
        vec![]
    };
    (None, Some(script), script_args)
}

// -----------------------------------------------------------------------------
// Script execution with shebang support.
// -----------------------------------------------------------------------------
pub fn execute_script(
    script_path: &str,
    script_args: &[String],
    state: &mut ShellState,
    rustybox_mode: bool,
    aliases: &mut HashMap<String, String>,
    functions: &mut HashMap<String, String>,
) -> u8 {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(script_path)
    {
        Ok(f) => f,
        Err(e) => {
            eprintln!("sh: {}: {}", script_path, e);
            return if e.kind() == io::ErrorKind::NotFound {
                127
            } else {
                126
            };
        }
    };
    let meta = match file.metadata() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("sh: {}: {}", script_path, e);
            return 126;
        }
    };
    if !meta.is_file() {
        eprintln!("sh: {}: not a regular file", script_path);
        return 126;
    }
    let mut content = String::with_capacity(4096);
    {
        let mut reader = BufReader::new(file.take(MAX_SCRIPT_BYTES as u64));
        if let Err(e) = reader.read_to_string(&mut content) {
            eprintln!("sh: {}: {}", script_path, e);
            return 126;
        }
    }
    if content.starts_with("#!") {
        let shebang_line = content.lines().next().unwrap_or("");
        let interpreter = shebang_line.trim_start_matches("#!").trim();
        if !interpreter.is_empty() {
            let parts: Vec<&str> = interpreter.split_whitespace().collect();
            let interpreter_path = parts[0];
            let interpreter_arg = if parts.len() > 1 { parts[1] } else { "" };
            let candidates: Vec<String> = if interpreter_path.starts_with('/') {
                if let Ok(canon) = Path::new(interpreter_path).canonicalize() {
                    let trusted = [
                        Path::new("/bin"),
                        Path::new("/usr/bin"),
                        Path::new("/usr/local/bin"),
                    ];
                    if trusted.iter().any(|t| canon.starts_with(t)) {
                        vec![canon.to_string_lossy().into_owned()]
                    } else {
                        eprintln!(
                            "sh: {}: interpreter '{}' not in trusted directories",
                            script_path, interpreter_path
                        );
                        return 126;
                    }
                } else {
                    eprintln!(
                        "sh: {}: cannot resolve interpreter path '{}'",
                        script_path, interpreter_path
                    );
                    return 126;
                }
            } else {
                if interpreter_path.contains('/') || interpreter_path.contains("..") {
                    eprintln!(
                        "sh: {}: insecure interpreter name '{}'",
                        script_path, interpreter_path
                    );
                    return 126;
                }
                ["/bin", "/usr/bin", "/usr/local/bin"]
                    .iter()
                    .map(|dir| format!("{}/{}", dir, interpreter_path))
                    .collect()
            };
            let mut argv_tokens: Vec<String> = Vec::new();
            if !interpreter_arg.is_empty() {
                argv_tokens.push(interpreter_arg.to_string());
            }
            argv_tokens.push(script_path.to_string());
            argv_tokens.extend_from_slice(script_args);
            for interp in &candidates {
                let mut full_tokens = vec![interp.clone()];
                full_tokens.extend_from_slice(&argv_tokens);
                let (argv_cstrings, argv_ptrs) = match build_argv(&full_tokens) {
                    Some(v) => v,
                    None => {
                        eprintln!("sh: invalid argument (null byte) in shebang invocation");
                        return 1;
                    }
                };
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
                        ACTIVE_CHILDREN.store(0, Ordering::SeqCst);
                        libc::execv(argv_ptrs[0], argv_ptrs.as_ptr());
                        libc::_exit(127);
                    }
                }
                drop(argv_cstrings);
                let status = crate::sh::exec::wait_for_child(pid);
                release_child_slot();
                if status != 127 {
                    return status;
                }
            }
            eprintln!(
                "sh: {}: shebang interpreter '{}' not found in trusted directories",
                script_path, interpreter_path
            );
            return 126;
        }
    }
    run_command_list(&content, state, 0, rustybox_mode, aliases, functions)
}
