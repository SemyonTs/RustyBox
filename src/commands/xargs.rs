// =============================================================================
// xargs — Build and execute command lines from standard input.
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
//   -n MAX   Use at most MAX arguments per command invocation.
//   -I REPL  Replace occurrences of REPL in the initial arguments with
//            each input item (one command per item).
//   -0       Input items are NUL-separated instead of whitespace-separated.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::io::{BufRead, Read};
use std::process::Command;

/// Entry point for the `xargs` builtin.
///
/// When no command is given `echo` is used as the default.
fn xargs_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "n:>0I:0") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("xargs: {e}");
            return 1;
        }
    };

    let max_args = opts.get_int('n').unwrap_or(0) as usize;
    let replace = opts.get_str('I').unwrap_or("").to_string();
    let flag_0 = opts.count('0') > 0;

    let mut args: Vec<String> = ctx.optargs.clone();

    // The first positional argument is the command; if absent, default to
    // `echo`.
    let command = if args.is_empty() {
        "echo".to_string()
    } else {
        args.remove(0)
    };
    let initial: Vec<String> = args;

    // Collect input items from stdin.
    let stdin = std::io::stdin();
    let mut items = Vec::new();

    if flag_0 {
        // NUL-delimited mode.
        let mut buf = Vec::new();
        stdin.lock().read_to_end(&mut buf).ok();
        for part in buf.split(|&b| b == 0) {
            if !part.is_empty() {
                items.push(String::from_utf8_lossy(part).into_owned());
            }
        }
    } else {
        // Whitespace-delimited mode.
        for line in stdin.lock().lines() {
            if let Ok(line) = line {
                for word in line.split_whitespace() {
                    items.push(word.to_string());
                }
            }
        }
    }

    let mut exit_code: u8 = 0;

    if !replace.is_empty() {
        // -I mode: one invocation per input item, with substitution.
        for item in &items {
            let mut cmd_args: Vec<String> = initial
                .iter()
                .map(|ia| ia.replace(&replace, item))
                .collect();
            // Also append the item itself when no replacement was found in
            // the initial arguments, to match traditional xargs behaviour.
            if !initial.iter().any(|ia| ia.contains(&replace)) {
                cmd_args.push(item.clone());
            }
            if let Err(e) = run_cmd(&command, &cmd_args) {
                eprintln!("xargs: {}", e);
                exit_code = 1;
            }
        }
    } else if max_args > 0 {
        // Chunked mode: at most `max_args` arguments per invocation.
        for chunk in items.chunks(max_args) {
            let mut cmd_args = initial.clone();
            cmd_args.extend_from_slice(chunk);
            if let Err(e) = run_cmd(&command, &cmd_args) {
                eprintln!("xargs: {}", e);
                exit_code = 1;
            }
        }
    } else {
        // Single invocation with all input items appended.
        let mut cmd_args = initial.clone();
        cmd_args.extend(items);
        if let Err(e) = run_cmd(&command, &cmd_args) {
            eprintln!("xargs: {}", e);
            exit_code = 1;
        }
    }

    exit_code
}

/// Spawn `command` with the given arguments and wait for it to finish.
///
/// Returns an error message if the command could not be started or exited
/// with a non-zero status.
fn run_cmd(command: &str, args: &[String]) -> Result<(), String> {
    let mut cmd = Command::new(command);
    cmd.args(args);
    let status = cmd.status().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("command exited with status {:?}", status.code()));
    }
    Ok(())
}

register_command!(
    XARGS_CMD,
    "xargs",
    "n:>0I:0",
    CommandFlags::BIN.bits(),
    xargs_main
);
