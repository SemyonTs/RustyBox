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
//   - Supports `--help`, `-h`, and `help <cmd>` for discoverability.
//   - Provides fuzzy suggestions on unknown commands.
// =============================================================================

use rustybox_utils::context::Context;
use rustybox_utils::registry;
use std::env;
use std::process::exit;

// ANSI escape codes for colored terminal output.
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

fn main() {
    let argv: Vec<String> = env::args().collect();
    if argv.is_empty() {
        exit(1);
    }

    // Determine the command name: strip any leading path from argv[0], or
    // use argv[1] when the binary is invoked directly as "rustybox".
    let argv0 = argv[0].rsplit('/').next().unwrap_or(&argv[0]);

    let (cmd_name, cmd_argv) = if argv0.starts_with("rustybox") || argv0.is_empty() {
        match argv.get(1).map(|s| s.as_str()) {
            // No arguments or explicit help flags: show general help.
            None | Some("--help") | Some("-h") => {
                print_help(None);
                exit(0);
            }
            // Support `rustybox help <cmd>` syntax.
            Some("help") => {
                let subcmd = argv.get(2).map(|s| s.as_str());
                print_help(subcmd);
                let code = if subcmd.is_some() && registry::find(subcmd.unwrap()).is_some() {
                    0
                } else {
                    1
                };
                exit(code);
            }
            Some(cmd) => (cmd, argv[1..].to_vec()),
        }
    } else {
        // Handle --help/-h even when invoked via symlink.
        if argv.iter().any(|a| a == "--help" || a == "-h") {
            print_help(Some(argv0));
            exit(0);
        }
        (argv0, argv.clone())
    };

    match registry::find(cmd_name) {
        Some(def) => {
            let mut ctx = Context::new(def, cmd_argv);
            let code = (def.run)(&mut ctx);
            exit(code as i32);
        }
        None => {
            eprintln!("{RED}error:{RESET} unknown command '{BOLD}{cmd_name}{RESET}'");

            // Suggest similar commands if available.
            if let Some(suggestion) = find_similar(cmd_name) {
                eprintln!();
                eprintln!("{YELLOW}tip:{RESET} did you mean '{GREEN}{suggestion}{RESET}'?");
            }

            eprintln!();
            eprintln!("Run '{BOLD}rustybox --help{RESET}' to see available commands.");
            exit(1);
        }
    }
}

/// Print help information. If `cmd` is provided, show help for that specific
/// command; otherwise print the general usage and command list.
fn print_help(cmd: Option<&str>) {
    if let Some(name) = cmd {
        if let Some(def) = registry::find(name) {
            println!("{BOLD}{}{RESET} {}", def.name, env!("CARGO_PKG_VERSION"));
            if let Some(desc) = def.description {
                println!("{desc}");
            }
            println!();

            if let Some(help) = def.help {
                print!("{help}");
                // Ensure trailing newline.
                if !help.ends_with('\n') {
                    println!();
                }
            } else {
                println!("{BOLD}USAGE:{RESET}");
                if def.optstr.is_empty() {
                    println!("  {} [ARGS...]", def.name);
                } else {
                    println!("  {} [OPTIONS] [ARGS...]", def.name);
                }
            }
            return;
        } else {
            eprintln!("{RED}error:{RESET} no help available for unknown command '{name}'");
            return;
        }
    }

    println!(
        "{BOLD}RustyBox {}{RESET} — some common *nix command-line utilities",
        env!("CARGO_PKG_VERSION")
    );
    println!();
    println!("{BOLD}USAGE:{RESET}");
    println!("  rustybox <COMMAND> [ARGS...]");
    println!("  rustybox help <COMMAND>");
    println!();
    println!("{BOLD}AVAILABLE COMMANDS:{RESET}");

    let cmds = registry::all();
    print_columns(&cmds);

    println!();
    println!("Run '{BOLD}rustybox help <COMMAND>{RESET}' for detailed usage.");
}

/// Format a command name for display. Names containing special characters
/// (like `[`) are quoted to avoid visual ambiguity in columnar output.
fn format_cmd_name(name: &str) -> String {
    if name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        name.to_string()
    } else {
        format!("'{name}'")
    }
}

/// Print commands in columns fitting the terminal width. Falls back to
/// 80 columns when terminal size cannot be determined.
fn print_columns(cmds: &[&rustybox_utils::registry::CommandDef]) {
    let term_width = terminal_width().unwrap_or(80);
    let indent = 2;
    let gap = 2;

    // Pre-format names and compute display widths.
    let formatted: Vec<(String, usize)> = cmds
        .iter()
        .map(|c| {
            let f = format_cmd_name(c.name);
            let w = f.len();
            (f, w)
        })
        .collect();

    let max_name_len = formatted.iter().map(|(_, w)| *w).max().unwrap_or(0);
    let col_width = max_name_len + gap;
    let cols = ((term_width - indent) / col_width).max(1);
    let rows = (formatted.len() + cols - 1) / cols;

    for row in 0..rows {
        print!("{:indent$}", "");
        for col in 0..cols {
            let idx = col * rows + row;
            if idx >= formatted.len() {
                break;
            }
            let (name, width) = &formatted[idx];
            if col < cols - 1 && idx + rows < formatted.len() {
                print!("{GREEN}{:<cw$}{RESET}", name, cw = col_width);
            } else {
                // Last column in this row: no trailing padding.
                print!("{GREEN}{name}{RESET}");
            }
        }
        println!();
    }
}

/// Try to get the terminal width. Returns None if stdout is not a tty
/// or the size cannot be queried.
fn terminal_width() -> Option<usize> {
    #[cfg(unix)]
    {
        use std::mem::MaybeUninit;
        unsafe {
            let mut ws: MaybeUninit<libc::winsize> = MaybeUninit::uninit();
            if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, ws.as_mut_ptr()) == 0 {
                let ws = ws.assume_init();
                if ws.ws_col > 0 {
                    return Some(ws.ws_col as usize);
                }
            }
        }
    }
    // Fallback: check COLUMNS env var.
    env::var("COLUMNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&w| w > 0)
}

/// Find the most similar registered command using Levenshtein distance.
/// Returns None if no command is within the similarity threshold.
fn find_similar(input: &str) -> Option<&'static str> {
    let mut best: Option<(&str, usize)> = None;
    let threshold = 3;

    for cmd in registry::all() {
        let dist = levenshtein(input, cmd.name);
        if dist <= threshold && best.map_or(true, |(_, b_dist)| dist < b_dist) {
            best = Some((cmd.name, dist));
        }
    }

    best.map(|(name, _)| name)
}

/// Compute the Levenshtein edit distance between two strings.
fn levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let m = a_chars.len();
    let n = b_chars.len();

    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }

    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0; n + 1];

    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = usize::from(a_chars[i - 1] != b_chars[j - 1]);
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[n]
}
