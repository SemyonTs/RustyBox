// =============================================================================
// main — Multicall binary entry point.
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Implementation inspired by Toybox (https://landley.net/toybox/)
// Toybox is copyrighted by Rob Landley, see NOTICE file for license details.
//
// Dispatch behaviour:
//   - When invoked via a symlink (e.g. `ls` → `rustybox`), `argv[0]`
//     determines the command.
//   - When invoked directly as `rustybox <cmd>`, the first argument
//     selects the command.
// =============================================================================

use rustybox::context::Context;
use rustybox::registry;
use std::env;
use std::process::exit;

fn main() {
    let argv: Vec<String> = env::args().collect();
    if argv.is_empty() {
        exit(1);
    }

    // Determine the command name: strip any leading path from argv[0], or
    // use argv[1] when the binary is invoked directly as "rustybox".
        let argv0 = argv[0].rsplit('/').next().unwrap_or(&argv[0]);
        let (cmd_name, cmd_argv) = if argv0.starts_with("rustybox") || argv0.is_empty() {
            if argv.len() < 2 {
                print_help();
                exit(0);
            }
            (argv[1].as_str(), argv[1..].to_vec())
        } else {
            (argv0, argv.clone())
        };

    match registry::find(cmd_name) {
        Some(def) => {
            let mut ctx = Context::new(def, cmd_argv);
            let code = (def.run)(&mut ctx);
            exit(code as i32);
        }
        None => {
            eprintln!("rustybox: unknown command '{}'", cmd_name);
            exit(1);
        }
    }
}

/// Print the list of all registered commands (analogous to `rustybox --help`).
fn print_help() {
    println!(
        "RustyBox {} — multicall binary",
        env!("CARGO_PKG_VERSION")
    );
    println!();
    println!("Available commands:");
    for cmd in registry::all() {
        println!("  {}", cmd.name);
    }
}