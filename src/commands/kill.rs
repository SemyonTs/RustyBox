// =============================================================================
// kill — Send a signal to a process.
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Implementation inspired by Toybox (https://landley.net/toybox/)
// Toybox is copyrighted by Rob Landley, see NOTICE file for license details.
//
// Supported options:
//   -l          List signal names, or translate a signal number to a name.
//   -s SIG      Specify the signal to send by name or number (default: SIGTERM).
//   -SIG        Shorthand for specifying the signal directly.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;

/// Entry point for the `kill` builtin.
///
/// The default signal is `SIGTERM` (15).
fn kill_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "ls:") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("kill: {e}");
            return 1;
        }
    };

    let flag_l = opts.count('l') > 0;
    let sig_name = opts.get_str('s').unwrap_or("").to_string();

    let args: Vec<String> = ctx.optargs.clone();

    // -l: list mode.
    if flag_l {
        if args.is_empty() {
            // Print all signal numbers and names.
            for i in 1..32 {
                if let Some(name) = signal_name(i) {
                    println!("{} {}", i, name);
                }
            }
        } else {
            // Translate each numeric argument to its signal name.
            for a in &args {
                if let Ok(n) = a.parse::<i32>() {
                    if let Some(name) = signal_name(n) {
                        println!("{}", name);
                    }
                }
            }
        }
        return 0;
    }

    // Determine which signal to send.
    let mut sig: i32 = 15; // SIGTERM
    let mut start = 0;

    if !sig_name.is_empty() {
        sig = signal_number(&sig_name).unwrap_or(15);
    } else if !args.is_empty() {
        let a = &args[0];
        if a.starts_with('-') {
            let s = &a[1..];
            if let Ok(n) = s.parse::<i32>() {
                sig = n;
                start = 1;
            } else {
                sig = signal_number(s).unwrap_or(15);
                start = 1;
            }
        }
    }

    // Send the signal to each listed PID.
    let mut exit_code: u8 = 0;
    for arg in &args[start..] {
        if let Ok(pid) = arg.parse::<i32>() {
            unsafe {
                if libc::kill(pid, sig) != 0 {
                    eprintln!("kill: ({}) {}", pid, std::io::Error::last_os_error());
                    exit_code = 1;
                }
            }
        } else {
            eprintln!("kill: invalid pid '{}'", arg);
            exit_code = 1;
        }
    }

    exit_code
}

/// Map a signal number (1–31) to its canonical name (e.g. `"SIGTERM"`).
///
/// Returns `None` for numbers outside the recognised range.
fn signal_name(n: i32) -> Option<&'static str> {
    Some(match n {
        1 => "SIGHUP",
        2 => "SIGINT",
        3 => "SIGQUIT",
        4 => "SIGILL",
        5 => "SIGTRAP",
        6 => "SIGABRT",
        7 => "SIGBUS",
        8 => "SIGFPE",
        9 => "SIGKILL",
        10 => "SIGUSR1",
        11 => "SIGSEGV",
        12 => "SIGUSR2",
        13 => "SIGPIPE",
        14 => "SIGALRM",
        15 => "SIGTERM",
        16 => "SIGSTKFLT",
        17 => "SIGCHLD",
        18 => "SIGCONT",
        19 => "SIGSTOP",
        20 => "SIGTSTP",
        21 => "SIGTTIN",
        22 => "SIGTTOU",
        23 => "SIGURG",
        24 => "SIGXCPU",
        25 => "SIGXFSZ",
        26 => "SIGVTALRM",
        27 => "SIGPROF",
        28 => "SIGWINCH",
        29 => "SIGIO",
        30 => "SIGPWR",
        31 => "SIGSYS",
        _ => return None,
    })
}

/// Resolve a signal specifier to its numeric value.
///
/// Accepts both names with or without the `SIG` prefix (e.g. `"TERM"` or
/// `"SIGTERM"`) and raw numeric strings.
fn signal_number(name: &str) -> Option<i32> {
    let n = name.trim_start_matches("SIG");
    for i in 1..32 {
        if let Some(sig_name) = signal_name(i) {
            if sig_name == name || sig_name.trim_start_matches("SIG") == n {
                return Some(i);
            }
        }
    }
    // Fallback: try to parse as a raw integer.
    name.parse::<i32>().ok()
}

register_command!(
    KILL_CMD,
    "kill",
    "ls:",
    CommandFlags::BIN.bits(),
    kill_main
);