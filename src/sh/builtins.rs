// =============================================================================
// sh builtins — cd, exit, exec, export, pwd, alias, jobs, fg, bg, source,
// eval, set, unset, and rustybox-mode command dispatch.
// =============================================================================
use crate::sh::exec::{run_command_list, run_external};
use crate::sh::expansion::expand_tilde_one;
use crate::sh::globals::*;
use crate::sh::signals::release_child_slot;
use std::collections::HashMap;
use std::env;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

// -----------------------------------------------------------------------------
// Builtin command dispatcher.
// -----------------------------------------------------------------------------
pub fn handle_builtin(
    tokens: &[String],
    state: &mut ShellState,
    depth: usize,
    aliases: &mut HashMap<String, String>,
    functions: &mut HashMap<String, String>,
) -> Option<u8> {
    if tokens.is_empty() {
        return Some(0);
    }
    match tokens[0].as_str() {
        "cd" => {
            let dir = if tokens.len() > 1 {
                expand_tilde_one(&tokens[1])
            } else {
                env::var("HOME").unwrap_or_else(|_| ".".into())
            };
            Some(cd_builtin(&dir))
        }
        "exit" => {
            let code = if tokens.len() > 1 {
                tokens[1].parse::<u8>().unwrap_or(0)
            } else {
                state.last_status
            };
            // Instead of calling std::process::exit (which skips destructors
            // and history saving), set a flag that the main loop checks.
            state.exit_requested = true;
            state.exit_code = code;
            Some(code)
        }
        "exec" => {
            if tokens.len() < 2 {
                eprintln!("exec: missing command");
                return Some(1);
            }
            let (args, argv) = match build_argv(&tokens[1..]) {
                Some(v) => v,
                None => {
                    eprintln!("exec: invalid argument (null byte)");
                    return Some(1);
                }
            };
            release_child_slot();
            unsafe {
                libc::execvp(argv[0], argv.as_ptr());
            }
            eprintln!("exec: command not found");
            drop(args);
            unsafe {
                libc::_exit(127);
            }
        }
        "export" => {
            for arg in &tokens[1..] {
                if let Some((key, value)) = arg.split_once('=') {
                    // Validate that the key is a legal POSIX identifier before
                    // setting it in the environment.
                    if !is_valid_identifier(key) {
                        eprintln!("export: '{}': not a valid identifier", key);
                        return Some(1);
                    }
                    // SAFETY: we assume single-threaded access to environ at
                    // this point (no concurrent readers in the shell process).
                    unsafe { env::set_var(key, value) };
                } else {
                    // Bare name: mark existing variable for export (no-op in
                    // this implementation since all env vars are inherited).
                    if !is_valid_identifier(arg) {
                        eprintln!("export: '{}': not a valid identifier", arg);
                        return Some(1);
                    }
                    if env::var(arg).is_err() {
                        eprintln!("export: {}: not a valid identifier", arg);
                        return Some(1);
                    }
                }
            }
            Some(0)
        }
        "pwd" => match env::current_dir() {
            Ok(p) => {
                println!("{}", p.display());
                Some(0)
            }
            Err(e) => {
                eprintln!("pwd: {e}");
                Some(1)
            }
        },
        "alias" => {
            if tokens.len() == 1 {
                for (name, value) in aliases.iter() {
                    println!("alias {}='{}'", name, value);
                }
            } else {
                for arg in &tokens[1..] {
                    if let Some((name, value)) = arg.split_once('=') {
                        aliases.insert(name.to_string(), value.trim_matches('\'').to_string());
                    } else if let Some(value) = aliases.get(arg) {
                        println!("alias {}='{}'", arg, value);
                    } else {
                        eprintln!("alias: {}: not found", arg);
                        return Some(1);
                    }
                }
            }
            Some(0)
        }
        "jobs" => Some(jobs_builtin()),
        "fg" => {
            let pid = tokens.get(1).and_then(|s| s.parse::<i32>().ok());
            Some(fg_builtin(pid))
        }
        "bg" => {
            let pid = tokens.get(1).and_then(|s| s.parse::<i32>().ok());
            Some(bg_builtin(pid))
        }
        "source" | "." => {
            if tokens.len() < 2 {
                eprintln!("{}: missing filename", tokens[0]);
                return Some(1);
            }
            let filename = expand_tilde_one(&tokens[1]);
            // Open with O_NOFOLLOW to prevent symlink following attacks.
            let file = match OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&filename)
            {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("{}: {}: {}", tokens[0], filename, e);
                    return Some(1);
                }
            };
            // Verify it is a regular file.
            let meta = match file.metadata() {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("{}: {}: {}", tokens[0], filename, e);
                    return Some(1);
                }
            };
            if !meta.is_file() {
                eprintln!("{}: {}: not a regular file", tokens[0], filename);
                return Some(126);
            }
            // Read with a hard limit to prevent TOCTOU.
            let mut content = String::with_capacity(4096);
            {
                let mut reader = BufReader::new(file.take(MAX_SCRIPT_BYTES as u64));
                if let Err(e) = reader.read_to_string(&mut content) {
                    eprintln!("{}: {}: {}", tokens[0], filename, e);
                    return Some(1);
                }
            }
            // Pass functions by mutable reference so that definitions
            // made inside the sourced file persist in the caller.
            let status = run_command_list(&content, state, depth + 1, false, aliases, functions);
            Some(status)
        }
        "eval" => {
            // POSIX: eval concatenates its arguments with spaces and re-parses
            // the result as a shell command.  This is intentional and expected
            // behaviour – the caller is responsible for proper quoting.
            let args = tokens[1..].join(" ");
            if args.len() > MAX_SCRIPT_BYTES {
                eprintln!("eval: command too long");
                return Some(1);
            }
            let status = run_command_list(&args, state, depth + 1, false, aliases, functions);
            Some(status)
        }
        "set" => {
            if tokens.len() > 1 {
                let mut idx = 1;
                while idx < tokens.len() {
                    let arg = &tokens[idx];
                    if arg == "-o" || arg == "+o" {
                        let enable = arg.starts_with('-');
                        if let Some(opt) = tokens.get(idx + 1) {
                            match opt.as_str() {
                                "pipefail" => {
                                    let shellopts = env::var("SHELLOPTS").unwrap_or_default();
                                    if enable {
                                        let new_val = if shellopts.is_empty() {
                                            "pipefail".to_string()
                                        } else if !shellopts.contains("pipefail") {
                                            format!("{} pipefail", shellopts)
                                        } else {
                                            shellopts
                                        };
                                        unsafe {
                                            env::set_var("SHELLOPTS", new_val);
                                        }
                                        state.pipefail = true;
                                    } else {
                                        let new_val = shellopts
                                            .split_whitespace()
                                            .filter(|w| *w != "pipefail")
                                            .collect::<Vec<_>>()
                                            .join(" ");
                                        unsafe {
                                            env::set_var("SHELLOPTS", new_val);
                                        }
                                        state.pipefail = false;
                                    }
                                }
                                _ => eprintln!("set: unknown option {}", opt),
                            }
                            idx += 1;
                        }
                    }
                    idx += 1;
                }
            } else {
                // Print shell variables and environment.
                println!("SHELL_STATE:");
                println!("  last_status={}", state.last_status);
                println!("  script_name={}", state.script_name);
                println!("  pipefail={}", state.pipefail);
                println!("  positional_params={:?}", state.positional_params);
                if let Some(bg) = state.last_bg_pid {
                    println!("  last_bg_pid={}", bg);
                }
                println!("ENVIRONMENT:");
                let mut vars: Vec<(String, String)> = env::vars().collect();
                vars.sort_by(|a, b| a.0.cmp(&b.0));
                for (k, v) in vars {
                    println!("{}={}", k, v);
                }
            }
            Some(0)
        }
        "unset" => {
            for arg in &tokens[1..] {
                if !is_valid_identifier(arg) {
                    eprintln!("unset: '{}': not a valid identifier", arg);
                    return Some(1);
                }
                unsafe { env::remove_var(arg) };
            }
            Some(0)
        }
        _ => None,
    }
}

pub fn handle_rustybox_builtin(tokens: &[String]) -> Option<u8> {
    if tokens.is_empty() {
        return None;
    }
    if matches!(
        tokens[0].as_str(),
        "cd" | "exit" | "exec" | "sh" | "export" | "pwd" | "alias"
    ) {
        return None;
    }
    let def = crate::registry::find(&tokens[0])?;
    let argv: Vec<String> = tokens.to_vec();
    let mut ctx = crate::context::Context::new(def, argv);
    let code = (def.run)(&mut ctx);
    Some(code)
}

// -----------------------------------------------------------------------------
// cd builtin with CDPATH support.
// -----------------------------------------------------------------------------
pub fn cd_builtin(dir: &str) -> u8 {
    let target = if dir == "-" {
        match env::var("OLDPWD") {
            Ok(old) => {
                println!("{old}");
                old
            }
            Err(_) => {
                eprintln!("cd: OLDPWD not set");
                return 1;
            }
        }
    } else {
        resolve_cdpath(dir)
    };
    let old_pwd = env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    if let Err(e) = env::set_current_dir(&target) {
        eprintln!("cd: {}: {}", target, e);
        return 1;
    }
    let new_pwd = env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or(target);
    unsafe { env::set_var("OLDPWD", old_pwd) };
    unsafe { env::set_var("PWD", &new_pwd) };
    0
}

pub fn resolve_cdpath(dir: &str) -> String {
    let p = Path::new(dir);
    if p.is_absolute() || p.starts_with(".") {
        return dir.to_string();
    }
    let cdpath = env::var("CDPATH").unwrap_or_default();
    if cdpath.is_empty() {
        return dir.to_string();
    }
    for entry in cdpath.split(':') {
        let candidate = if entry.is_empty() {
            Path::new(dir).to_path_buf()
        } else {
            Path::new(entry).join(dir)
        };
        if candidate.is_dir() {
            return candidate.to_string_lossy().into_owned();
        }
    }
    dir.to_string()
}

// -----------------------------------------------------------------------------
// Job control builtins.
// -----------------------------------------------------------------------------
pub fn jobs_builtin() -> u8 {
    let jobs = JOBS.lock().unwrap();
    for job in jobs.iter() {
        let status = match job.state {
            JobState::Running => "Running",
            JobState::Stopped => "Stopped",
        };
        println!("[{}] {} - {}", job.pid, status, job.command);
    }
    0
}

pub fn fg_builtin(pid: Option<i32>) -> u8 {
    // Acquire the job table lock for the whole operation to narrow the PID
    // recycling window (though not completely eliminating it on all platforms).
    let jobs = JOBS.lock().unwrap();
    let target_pid = match pid {
        Some(p) => {
            if !jobs
                .iter()
                .any(|j| j.pid == p && unsafe { libc::kill(p, 0) == 0 })
            {
                eprintln!("fg: {}: no such job", p);
                return 1;
            }
            p
        }
        None => match jobs.last() {
            Some(j) if unsafe { libc::kill(j.pid, 0) == 0 } => j.pid,
            _ => {
                eprintln!("fg: no current job");
                return 1;
            }
        },
    };
    let pgid = jobs
        .iter()
        .find(|j| j.pid == target_pid)
        .map(|j| j.pgid)
        .unwrap_or(target_pid);
    // Drop the lock before performing blocking wait, but keep the pgid.
    // On Linux, a pidfd could be used for stronger guarantee; here we accept
    // a residual race.
    drop(jobs);
    // Re-check that the process is still alive before proceeding.
    if unsafe { libc::kill(target_pid, 0) } != 0 {
        eprintln!("fg: process {} terminated", target_pid);
        return 1;
    }
    unsafe {
        libc::signal(libc::SIGTTOU, libc::SIG_IGN);
        libc::tcsetpgrp(libc::STDIN_FILENO, pgid);
    }
    crate::sh::exec::update_job_state(target_pid, JobState::Running);
    unsafe {
        libc::kill(-pgid, libc::SIGCONT);
    }
    let status = crate::sh::exec::wait_for_child(target_pid);
    unsafe {
        libc::tcsetpgrp(libc::STDIN_FILENO, libc::getpgrp());
        libc::signal(libc::SIGTTOU, libc::SIG_DFL);
    }
    status
}

pub fn bg_builtin(pid: Option<i32>) -> u8 {
    let jobs = JOBS.lock().unwrap();
    let target_pid = match pid {
        Some(p) => {
            if !jobs
                .iter()
                .any(|j| j.pid == p && unsafe { libc::kill(p, 0) == 0 })
            {
                eprintln!("bg: {}: no such job", p);
                return 1;
            }
            p
        }
        None => match jobs.last() {
            Some(j) if unsafe { libc::kill(j.pid, 0) == 0 } => j.pid,
            _ => {
                eprintln!("bg: no current job");
                return 1;
            }
        },
    };
    let pgid = jobs
        .iter()
        .find(|j| j.pid == target_pid)
        .map(|j| j.pgid)
        .unwrap_or(target_pid);
    drop(jobs);
    if unsafe { libc::kill(target_pid, 0) } != 0 {
        eprintln!("bg: process {} terminated", target_pid);
        return 1;
    }
    crate::sh::exec::update_job_state(target_pid, JobState::Running);
    unsafe {
        libc::kill(-pgid, libc::SIGCONT);
    }
    eprintln!("[{}] continued in background", target_pid);
    0
}