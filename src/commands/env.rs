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
//   -u NAME     Remove the variable NAME from the environment (extension).
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
    let opts = match crate::args::parse(ctx, "i") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("env: {e}");
            return 1;
        }
    };

    let flag_i = opts.count('i') > 0;

    // Build the target environment.
    let mut new_env: HashMap<String, String> = if flag_i {
        HashMap::new()
    } else {
        std::env::vars().collect()
    };

    // Process leading environment modifiers in the order they appear:
    //   name=value     — set (or override) an environment variable.
    //   -u NAME        — remove NAME (comma-separated list accepted).
    //   -uNAME         — equivalent form.
    //   --             — end of modifiers; everything after is the command.
    let mut i = 0;
    while i < ctx.optargs.len() {
        let arg = &ctx.optargs[i];
        if arg == "--" {
            i += 1; // skip the delimiter
            break;
        } else if arg == "-u" {
            i += 1;
            if i >= ctx.optargs.len() {
                eprintln!("env: option requires an argument -- 'u'");
                return 125;
            }
            let unset_arg = &ctx.optargs[i];
            for name in unset_arg.split(',') {
                if !name.is_empty() {
                    new_env.remove(name);
                }
            }
            i += 1;
        } else if let Some(stripped) = arg.strip_prefix("-u") {
            // -uNAME form (e.g. -uPATH or -uPATH,HOME)
            for name in stripped.split(',') {
                if !name.is_empty() {
                    new_env.remove(name);
                }
            }
            i += 1;
        } else if let Some((name, value)) = arg.split_once('=') {
            if !name.is_empty() {
                new_env.insert(name.to_string(), value.to_string());
                i += 1;
            } else {
                // Empty name is invalid; treat as start of command.
                break;
            }
        } else {
            // Any other argument (including those starting with '-')
            // is the start of the utility and its arguments.
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
    // Ensure only the variables in `new_env` are present.
    cmd.env_clear();
    for (k, v) in &new_env {
        cmd.env(k, v);
    }

    match cmd.status() {
        Ok(status) => {
            if let Some(code) = status.code() {
                code as u8
            } else {
                // Terminated by signal; propagate signal number + 128.
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    if let Some(signal) = status.signal() {
                        128u8.saturating_add(signal as u8)
                    } else {
                        1
                    }
                }
                #[cfg(not(unix))]
                1
            }
        }
        Err(e) => {
            eprintln!("env: {}", e);
            if e.kind() == std::io::ErrorKind::NotFound {
                127
            } else {
                126
            }
        }
    }
}

register_command!(
    ENV_CMD,
    "env",
    "i",
    CommandFlags::BIN.bits(),
    env_main,
    description = "Run a command in a modified environment",
    help = "\
OPTIONS:
-i          Start with an empty environment, ignoring the inherited one.
-u NAME     Remove the variable NAME from the environment (extension).
NAME=VALUE  Set (or override) an environment variable."
);
