// =============================================================================
// env — Run a command in a modified environment.
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
//   -i          Start with an empty environment, ignoring the inherited one.
//   -u NAME     Remove the variable NAME from the environment.
//   NAME=VALUE  Set (or override) an environment variable.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::collections::HashMap;
use std::process::Command;

/// Entry point for the `env` builtin.
///
/// When no command is given the current environment is printed to stdout,
/// one `KEY=VALUE` pair per line, sorted by key.
fn env_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "iu:") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("env: {e}");
            return 1;
        }
    };

    let flag_i = opts.count('i') > 0;
    let unset_raw = opts.get_str('u').unwrap_or("");

    // Build the target environment.
    let mut new_env: HashMap<String, String> = if flag_i {
        HashMap::new()
    } else {
        std::env::vars().collect()
    };

    // Drop variables requested via -u (comma-separated list).
    // Use split() directly on the borrowed &str — no allocation.
    for name in unset_raw.split(',') {
        if !name.is_empty() {
            new_env.remove(name);
        }
    }

    // Consume leading NAME=VALUE pairs from the positional arguments.
    // Use an index into ctx.optargs instead of cloning the whole vector.
    let mut i = 0;
    while i < ctx.optargs.len() {
        let arg = &ctx.optargs[i];
        if let Some((name, value)) = arg.split_once('=') {
            if !name.is_empty() {
                new_env.insert(name.to_string(), value.to_string());
                i += 1;
            } else {
                break;
            }
        } else {
            break;
        }
    }

    // Remaining arguments form the command to execute.
    let cmd_args = &ctx.optargs[i..];

    if cmd_args.is_empty() {
        // No command — print the environment and exit.
        // Collect keys into a Vec<&String> and sort in-place to avoid
        // allocating a second Vec for the sorted copy.
        let mut keys: Vec<&String> = new_env.keys().collect();
        keys.sort();
        for k in keys {
            println!("{}={}", k, new_env[k.as_str()]);
        }
        return 0;
    }

    // Spawn the requested command with the modified environment.
    let mut cmd = Command::new(&cmd_args[0]);
    cmd.args(&cmd_args[1..]);
    for (k, v) in &new_env {
        cmd.env(k, v);
    }

    match cmd.status() {
        Ok(status) => {
            if let Some(code) = status.code() {
                code as u8
            } else {
                1
            }
        }
        Err(e) => {
            eprintln!("env: {}", e);
            127
        }
    }
}

register_command!(ENV_CMD, "env", "iu:", CommandFlags::BIN.bits(), env_main);
