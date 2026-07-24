// =============================================================================
// sh — POSIX-compliant command interpreter.
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Supported features:
//   - Command execution with PATH resolution.
//   - Builtin commands: cd, exit, exec.
//   - I/O redirection: <, >, >>.
//   - Pipelines: |.
//   - Signal handling: SIGINT, SIGQUIT, SIGTSTP ignored by shell; SIGINT
//     forwarded to foreground child.
//   - Exit status: $? from last command.
//   - sh rustybox: all rustybox commands available as builtins via registry::find.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use crate::registry;
use std::env;
use std::ffi::CString;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::io::{FromRawFd, IntoRawFd, RawFd};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::exit;
use std::{fs::OpenOptions, path::PathBuf};

/// Maximum length of a single input line.
const LINE_MAX: usize = 4096;

/// Entry point for the `sh` builtin.
fn sh_main(ctx: &mut Context) -> u8 {
    let _opts = match crate::args::parse(ctx, "") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("sh: {e}");
            return 1;
        }
    };

    let rustybox_mode = ctx
        .optargs
        .first()
        .map(|s| s == "rustybox")
        .unwrap_or(false);

    unsafe {
        libc::signal(libc::SIGINT, libc::SIG_IGN);
        libc::signal(libc::SIGQUIT, libc::SIG_IGN);
        libc::signal(libc::SIGTSTP, libc::SIG_IGN);
        libc::signal(libc::SIGCHLD, libc::SIG_DFL);
    }

    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let mut line = String::with_capacity(LINE_MAX);
    let mut last_status: u8 = 0;

    loop {
        if is_interactive() {
            eprint!("$ ");
            io::stderr().flush().ok();
        }

        line.clear();
        let n = match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };

        let input = line[..n].trim();
        if input.is_empty() {
            continue;
        }

        last_status = run_command(input, last_status, rustybox_mode);
    }

    last_status
}

fn is_interactive() -> bool {
    unsafe { libc::isatty(libc::STDIN_FILENO) != 0 }
}

fn run_command(input: &str, _last_status: u8, rustybox_mode: bool) -> u8 {
    let segments: Vec<&str> = input.split('|').collect();

    if segments.len() == 1 {
        return run_simple_command(segments[0], rustybox_mode);
    }

    run_pipeline(&segments, rustybox_mode)
}

fn run_simple_command(input: &str, rustybox_mode: bool) -> u8 {
    let tokens = tokenize(input);
    if tokens.is_empty() {
        return 0;
    }

    let (tokens, stdin_file, stdout_file, append) = parse_redirections(tokens);

    // Shell builtins first.
    if let Some(result) = handle_builtin(&tokens) {
        return result;
    }

    // Rustybox builtin (only outside pipelines).
    if rustybox_mode {
        if let Some(result) = handle_rustybox_builtin(&tokens) {
            return result;
        }
    }

    // External command.
    run_external(&tokens, stdin_file, stdout_file, append, rustybox_mode)
}

fn run_pipeline(segments: &[&str], rustybox_mode: bool) -> u8 {
    let mut prev_read_fd: Option<RawFd> = None;
    let mut pids: Vec<i32> = Vec::new();
    let mut last_status: u8 = 0;

    for (i, segment) in segments.iter().enumerate() {
        let tokens = tokenize(segment);
        if tokens.is_empty() {
            continue;
        }

        let is_last = i == segments.len() - 1;
        let (tokens, stdin_file, stdout_file, append) = if is_last || i == 0 {
            parse_redirections(tokens)
        } else {
            (tokens, None, None, false)
        };

        // Only pure shell builtins are rejected in pipelines.
        if tokens[0] == "cd" || tokens[0] == "exit" || tokens[0] == "exec" {
            eprintln!("sh: '{}' cannot be used in pipelines", tokens[0]);
            return 1;
        }

        let (pipe_read, pipe_write) = if !is_last {
            let fds = pipe().unwrap();
            (Some(fds.0), Some(fds.1))
        } else {
            (None, None)
        };

        let pid = unsafe { libc::fork() };
        if pid < 0 {
            eprintln!("sh: fork failed");
            return 1;
        }

        if pid == 0 {
            unsafe {
                // In pipeline children, ignore SIGPIPE so they exit cleanly
                // when the downstream command closes the pipe.
                libc::signal(libc::SIGPIPE, libc::SIG_IGN);

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

                if let Some(f) = &stdin_file {
                    if let Ok(fd) = open_file(f, false) {
                        libc::dup2(fd, libc::STDIN_FILENO);
                        libc::close(fd);
                    }
                }
                if let Some(f) = &stdout_file {
                    if let Ok(fd) = open_file(f, append) {
                        libc::dup2(fd, libc::STDOUT_FILENO);
                        libc::close(fd);
                    }
                }

                libc::signal(libc::SIGINT, libc::SIG_DFL);
                libc::signal(libc::SIGQUIT, libc::SIG_DFL);

                // Try rustybox builtin in child process.
                if rustybox_mode {
                    if let Some(def) = registry::find(&tokens[0]) {
                        let argv: Vec<String> = tokens.to_vec();
                        let mut ctx = Context::new(def, argv);
                        let code = (def.run)(&mut ctx);
                        libc::_exit(code as i32);
                    }
                }

                let args: Vec<CString> = tokens
                    .iter()
                    .map(|s| CString::new(s.as_bytes()).unwrap())
                    .collect();
                let argv: Vec<*const libc::c_char> = args
                    .iter()
                    .map(|s| s.as_ptr())
                    .chain(std::iter::once(std::ptr::null()))
                    .collect();
                libc::execvp(argv[0], argv.as_ptr());
                libc::_exit(127);
            }
        }

        pids.push(pid);

        if let Some(read_fd) = prev_read_fd {
            unsafe {
                libc::close(read_fd);
            }
        }
        if let Some(read_fd) = pipe_read {
            unsafe {
                libc::close(read_fd);
            }
        }

        prev_read_fd = pipe_read;

        if let Some(write_fd) = pipe_write {
            unsafe {
                libc::close(write_fd);
            }
        }
    }

    if let Some(read_fd) = prev_read_fd {
        unsafe {
            libc::close(read_fd);
        }
    }

    for &pid in &pids {
        let mut status: i32 = 0;
        unsafe {
            libc::waitpid(pid, &mut status, 0);
            if libc::WIFEXITED(status) {
                last_status = libc::WEXITSTATUS(status) as u8;
            }
        }
    }

    last_status
}

/// Handle shell builtin commands (cd, exit, exec).
fn handle_builtin(tokens: &[String]) -> Option<u8> {
    if tokens.is_empty() {
        return Some(0);
    }

    match tokens[0].as_str() {
        "cd" => {
            let dir = if tokens.len() > 1 {
                tokens[1].clone()
            } else {
                match env::var("HOME") {
                    Ok(h) => h,
                    Err(_) => {
                        eprintln!("cd: HOME not set");
                        return Some(1);
                    }
                }
            };
            Some(cd_builtin(&dir))
        }
        "exit" => {
            let code = if tokens.len() > 1 {
                tokens[1].parse::<u8>().unwrap_or(0)
            } else {
                0
            };
            exit(code as i32);
        }
        "exec" => {
            if tokens.len() < 2 {
                eprintln!("exec: missing command");
                return Some(1);
            }
            let args: Vec<CString> = tokens[1..]
                .iter()
                .map(|s| CString::new(s.as_bytes()).unwrap())
                .collect();
            let argv: Vec<*const libc::c_char> = args
                .iter()
                .map(|s| s.as_ptr())
                .chain(std::iter::once(std::ptr::null()))
                .collect();
            unsafe {
                libc::execvp(argv[0], argv.as_ptr());
            }
            eprintln!("exec: command not found");
            exit(127);
        }
        _ => None,
    }
}

/// Handle a rustybox command as a builtin using on‑demand registry lookup.
fn handle_rustybox_builtin(tokens: &[String]) -> Option<u8> {
    if tokens.is_empty() {
        return None;
    }

    if tokens[0] == "cd" || tokens[0] == "exit" || tokens[0] == "exec" || tokens[0] == "sh" {
        return None;
    }

    let def = registry::find(&tokens[0])?;

    let argv: Vec<String> = tokens.to_vec();
    let mut ctx = Context::new(def, argv);
    let code = (def.run)(&mut ctx);
    Some(code)
}

/// Run an external command via fork + execvp.
fn run_external(
    tokens: &[String],
    stdin_file: Option<String>,
    stdout_file: Option<String>,
    append: bool,
    rustybox_mode: bool,
) -> u8 {
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        eprintln!("sh: fork failed");
        return 1;
    }

    if pid == 0 {
        unsafe {
            if let Some(f) = &stdin_file {
                if let Ok(fd) = open_file(f, false) {
                    libc::dup2(fd, libc::STDIN_FILENO);
                    libc::close(fd);
                }
            }
            if let Some(f) = &stdout_file {
                if let Ok(fd) = open_file(f, append) {
                    libc::dup2(fd, libc::STDOUT_FILENO);
                    libc::close(fd);
                }
            }

            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::signal(libc::SIGQUIT, libc::SIG_DFL);

            // Try rustybox builtin in child process.
            if rustybox_mode {
                if let Some(def) = registry::find(&tokens[0]) {
                    let argv: Vec<String> = tokens.to_vec();
                    let mut ctx = Context::new(def, argv);
                    let code = (def.run)(&mut ctx);
                    libc::_exit(code as i32);
                }
            }

            let args: Vec<CString> = tokens
                .iter()
                .map(|s| CString::new(s.as_bytes()).unwrap())
                .collect();
            let argv: Vec<*const libc::c_char> = args
                .iter()
                .map(|s| s.as_ptr())
                .chain(std::iter::once(std::ptr::null()))
                .collect();
            libc::execvp(argv[0], argv.as_ptr());
            libc::_exit(127);
        }
    }

    let mut status: i32 = 0;
    unsafe {
        libc::waitpid(pid, &mut status, 0);
        if libc::WIFEXITED(status) {
            libc::WEXITSTATUS(status) as u8
        } else {
            1
        }
    }
}

/// Builtin cd implementation with POSIX semantics.
fn cd_builtin(dir: &str) -> u8 {
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

    let old_pwd = env::var("PWD").unwrap_or_else(|_| {
        env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default()
    });

    if let Err(e) = env::set_current_dir(&target) {
        eprintln!("cd: {}: {}", target, e);
        return 1;
    }

    let new_pwd = env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or(target);

    unsafe {
        env::set_var("OLDPWD", old_pwd);
        env::set_var("PWD", &new_pwd);
    }

    0
}

/// Resolve a directory using $CDPATH (POSIX).
fn resolve_cdpath(dir: &str) -> String {
    let p = Path::new(dir);
    if p.is_absolute() || p.starts_with(".") {
        return dir.to_string();
    }

    let cdpath = match env::var("CDPATH") {
        Ok(v) => v,
        Err(_) => return dir.to_string(),
    };
    if cdpath.is_empty() {
        return dir.to_string();
    }

    for entry in cdpath.split(':') {
        if entry.is_empty() {
            if Path::new(dir).is_dir() {
                return dir.to_string();
            }
            continue;
        }
        let candidate = Path::new(entry).join(dir);
        if candidate.is_dir() {
            return candidate.to_string_lossy().into_owned();
        }
    }

    dir.to_string()
}

/// Tokenize a command string into words, respecting single and double quotes.
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    let mut current = String::with_capacity(64);

    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }

        current.clear();

        if bytes[i] == b'\'' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'\'' {
                current.push(bytes[i] as char);
                i += 1;
            }
            if i < bytes.len() {
                i += 1;
            }
        } else if bytes[i] == b'"' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 1;
                    current.push(bytes[i] as char);
                } else {
                    current.push(bytes[i] as char);
                }
                i += 1;
            }
            if i < bytes.len() {
                i += 1;
            }
        } else {
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                current.push(bytes[i] as char);
                i += 1;
            }
        }

        if !current.is_empty() {
            tokens.push(current.clone());
        }
    }

    tokens
}

/// Parse I/O redirection operators: `<`, `>`, `>>`.
fn parse_redirections(
    mut tokens: Vec<String>,
) -> (Vec<String>, Option<String>, Option<String>, bool) {
    let mut stdin_file = None;
    let mut stdout_file = None;
    let mut append = false;
    let mut result = Vec::with_capacity(tokens.len());

    let mut i = 0;
    while i < tokens.len() {
        if tokens[i] == "<" && i + 1 < tokens.len() {
            stdin_file = Some(tokens[i + 1].clone());
            i += 2;
        } else if tokens[i] == ">" && i + 1 < tokens.len() {
            stdout_file = Some(tokens[i + 1].clone());
            append = false;
            i += 2;
        } else if tokens[i] == ">>" && i + 1 < tokens.len() {
            stdout_file = Some(tokens[i + 1].clone());
            append = true;
            i += 2;
        } else {
            result.push(tokens[i].clone());
            i += 1;
        }
    }

    (result, stdin_file, stdout_file, append)
}

/// Create a pipe: (read_fd, write_fd).
fn pipe() -> io::Result<(RawFd, RawFd)> {
    let mut fds = [-1i32; 2];
    let rc = unsafe { libc::pipe(fds.as_mut_ptr()) };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok((fds[0], fds[1]))
    }
}

/// Open a file for redirection.
fn open_file(path: &str, append: bool) -> io::Result<RawFd> {
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .append(append)
        .truncate(!append)
        .open(path)?;
    Ok(file.into_raw_fd())
}

register_command!(
    SH_CMD,
    "sh",
    "",
    CommandFlags::BIN.bits() | CommandFlags::NOFORK.bits(),
    sh_main
);
