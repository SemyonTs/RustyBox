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
fn kill_main(ctx: &mut Context) -> u8 {
    // We manually parse ctx.argv to support POSIX/XSI shorthand syntax (-N, -NAME)
    // which standard parsers often reject or mishandle.
    let mut explicit_signal: Option<i32> = None;
    let mut list_mode = false;
    let mut pids = Vec::new();

    // Skip argv[0] (command name)
    let mut i = 1;
    while i < ctx.argv.len() {
        let arg = &ctx.argv[i];

        if arg == "--" {
            // End of options, rest are PIDs
            i += 1;
            while i < ctx.argv.len() {
                pids.push(ctx.argv[i].clone());
                i += 1;
            }
            break;
        } else if arg == "-l" {
            list_mode = true;
            i += 1;
        } else if arg == "-s" {
            i += 1;
            if i < ctx.argv.len() {
                let sig_arg = &ctx.argv[i];
                explicit_signal = signal_number(sig_arg);
                if explicit_signal.is_none() {
                    eprintln!("kill: invalid signal '{}'", sig_arg);
                    return 1;
                }
            } else {
                eprintln!("kill: missing argument for -s");
                return 1;
            }
            i += 1;
        } else if arg.starts_with("-s") {
            // Handle -sVALUE format
            let val = &arg[2..];
            explicit_signal = signal_number(val);
            if explicit_signal.is_none() {
                eprintln!("kill: invalid signal '{}'", val);
                return 1;
            }
            i += 1;
        } else if arg.starts_with('-') && arg.len() > 1 {
            // Shorthand: -N or -NAME (e.g., -TERM, -9, -0)
            let part = &arg[1..];
            let sig_num = signal_number(part);
            if let Some(num) = sig_num {
                explicit_signal = Some(num);
            } else {
                eprintln!("kill: unknown option/signal '{}'", arg);
                return 1;
            }
            i += 1;
        } else {
            // It's a PID or an argument for -l
            pids.push(arg.clone());
            i += 1;
        }
    }

    // --- List Mode Logic ---
    if list_mode {
        if pids.is_empty() {
            // Print all signal names separated by spaces (POSIX format)
            let mut first = true;
            for i in 1..32 {
                if let Some(name) = signal_name(i) {
                    if !first {
                        print!(" ");
                    }
                    print!("{}", name);
                    first = false;
                }
            }
            println!();
        } else {
            // Translate arguments: if number -> name, if name -> number
            for a in &pids {
                if let Ok(n) = a.parse::<i32>() {
                    if let Some(name) = signal_name(n) {
                        println!("{}", name);
                    } else {
                        eprintln!("kill: invalid signal number '{}'", n);
                        return 1;
                    }
                } else {
                    // Try as name
                    if let Some(num) = signal_number(a) {
                        println!("{}", num);
                    } else {
                        eprintln!("kill: invalid signal name '{}'", a);
                        return 1;
                    }
                }
            }
        }
        return 0;
    }

    // --- Send Signal Logic ---
    let sig = explicit_signal.unwrap_or(15); // Default SIGTERM

    if pids.is_empty() {
        eprintln!("kill: usage: kill [-s sig] pid...");
        return 1;
    }

    let mut exit_code: u8 = 0;
    for arg in &pids {
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

/// Map a signal number to its canonical name (without SIG prefix for POSIX compliance).
fn signal_name(n: i32) -> Option<&'static str> {
    Some(match n {
        1 => "HUP",
        2 => "INT",
        3 => "QUIT",
        4 => "ILL",
        5 => "TRAP",
        6 => "ABRT",
        7 => "BUS",
        8 => "FPE",
        9 => "KILL",
        10 => "USR1",
        11 => "SEGV",
        12 => "USR2",
        13 => "PIPE",
        14 => "ALRM",
        15 => "TERM",
        16 => "STKFLT",
        17 => "CHLD",
        18 => "CONT",
        19 => "STOP",
        20 => "TSTP",
        21 => "TTIN",
        22 => "TTOU",
        23 => "URG",
        24 => "XCPU",
        25 => "XFSZ",
        26 => "VTALRM",
        27 => "PROF",
        28 => "WINCH",
        29 => "IO",
        30 => "PWR",
        31 => "SYS",
        _ => return None,
    })
}

/// Resolve a signal specifier to its numeric value.
/// Accepts names with or without the `SIG` prefix (e.g. `"TERM"`, `"SIGTERM"`) and raw numbers.
fn signal_number(name: &str) -> Option<i32> {
    let n = name.trim_start_matches("SIG");
    for i in 1..32 {
        if let Some(sig_name) = signal_name(i) {
            if sig_name == n {
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
    "",
    CommandFlags::BIN.bits(),
    kill_main,
    description = "Send a signal to a process",
    help = "\
OPTIONS:
-l          List signal names, or translate a signal number to a name.
-s SIG      Specify the signal to send by name or number (default: SIGTERM).
-SIG        Shorthand for specifying the signal directly."
);
