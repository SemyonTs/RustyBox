// =============================================================================
// sed — Stream editor for filtering and transforming text.
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Implementation inspired by Toybox (https://landley.net/toybox/)
// Toybox is copyrighted by Rob Landley, see NOTICE file for license details.
//
// Supported commands:
//   s/pattern/replacement/[g]   Substitute (global with `g` flag).
//   p                           Print the current pattern space.
//   d                           Delete the pattern space; start next cycle.
//   a text                      Append text after the current line.
//   i text                      Insert text before the current line.
//   c text                      Replace the current line with text.
//
// Supported options:
//   -n        Suppress automatic printing of the pattern space.
//   -e SCRIPT Add a script to the list of commands (may be repeated).
//   -f FILE   Read script commands from FILE.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use regex::Regex;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};

/// A single parsed sed command.
enum Command {
    Substitute {
        pattern: Regex,
        repl: String,
        global: bool,
    },
    Print,
    Delete,
    Append(String),
    Insert(String),
    Change(String),
}

/// Entry point for the `sed` builtin.
///
/// Scripts are accumulated from `-e` options, `-f` files, and (if neither is
/// supplied) the first positional argument.  Multiple scripts are
/// concatenated with `;` separators.
fn sed_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "ne:f:") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("sed: {e}");
            return 1;
        }
    };

    let flag_n = opts.count('n') > 0;

    let mut scripts: Vec<String> = Vec::new();

    if let Some(e) = opts.get_str('e') {
        if !e.is_empty() {
            scripts.push(e.to_string());
        }
    }

    if let Some(f) = opts.get_str('f') {
        if let Ok(file) = File::open(f) {
            for line in BufReader::new(file).lines() {
                if let Ok(l) = line {
                    scripts.push(l);
                }
            }
        }
    }

    let mut args: Vec<String> = ctx.optargs.clone();
    if scripts.is_empty() {
        if args.is_empty() {
            eprintln!("sed: no script specified");
            return 1;
        }
        scripts.push(args.remove(0));
    }

    // Compile all scripts into a flat list of commands.
    let mut cmds = Vec::new();
    for script in &scripts {
        match parse_script(script) {
            Ok(mut c) => cmds.append(&mut c),
            Err(e) => {
                eprintln!("sed: {e}");
                return 1;
            }
        }
    }

    let files: Vec<String> = if args.is_empty() {
        vec!["-".to_string()]
    } else {
        args
    };

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut exit_code: u8 = 0;

    for file in &files {
        let reader: Box<dyn BufRead> = if file == "-" {
            Box::new(std::io::stdin().lock())
        } else {
            match File::open(file) {
                Ok(f) => Box::new(BufReader::new(f)),
                Err(e) => {
                    eprintln!("sed: '{}': {}", file, e);
                    exit_code = 1;
                    continue;
                }
            }
        };

        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };
            if let Err(e) = process_line(&line, &cmds, flag_n, &mut out) {
                eprintln!("sed: {e}");
                exit_code = 1;
            }
        }
    }

    exit_code
}

/// Parse a script string (possibly containing `;`-separated commands) into a
/// vector of `Command` values.
fn parse_script(script: &str) -> Result<Vec<Command>, String> {
    let mut cmds = Vec::new();
    for part in script.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        cmds.push(parse_command(part)?);
    }
    Ok(cmds)
}

/// Parse a single command (e.g. `s/foo/bar/g`, `p`, `d`, `a text`).
fn parse_command(s: &str) -> Result<Command, String> {
    if let Some(rest) = s.strip_prefix('s') {
        let delim = rest.chars().next().ok_or("empty s command")?;
        let parts: Vec<&str> = rest[1..].split(delim).collect();
        if parts.len() < 2 {
            return Err("malformed s command".to_string());
        }
        let pattern = Regex::new(parts[0]).map_err(|e| e.to_string())?;
        let repl = parts[1].to_string();
        let global = parts.get(2).map(|f| f.contains('g')).unwrap_or(false);
        return Ok(Command::Substitute {
            pattern,
            repl,
            global,
        });
    }

    if s == "p" {
        return Ok(Command::Print);
    }

    if s == "d" {
        return Ok(Command::Delete);
    }

    if let Some(text) = s.strip_prefix('a') {
        return Ok(Command::Append(text.trim().to_string()));
    }

    if let Some(text) = s.strip_prefix('i') {
        return Ok(Command::Insert(text.trim().to_string()));
    }

    if let Some(text) = s.strip_prefix('c') {
        return Ok(Command::Change(text.trim().to_string()));
    }

    Err(format!("unknown command '{}'", s))
}

/// Apply the list of commands to a single input line and write the result
/// to `out`.
fn process_line<W: Write>(
    line: &str,
    cmds: &[Command],
    flag_n: bool,
    out: &mut W,
) -> Result<(), String> {
    let mut current = line.to_string();
    let mut deleted = false;
    let mut printed = false;

    for cmd in cmds {
        match cmd {
            Command::Substitute {
                pattern,
                repl,
                global,
            } => {
                if *global {
                    current = pattern
                        .replace_all(&current, repl.as_str())
                        .to_string();
                } else {
                    current = pattern.replace(&current, repl.as_str()).to_string();
                }
            }
            Command::Print => {
                writeln!(out, "{}", current).map_err(|e| e.to_string())?;
                printed = true;
            }
            Command::Delete => {
                deleted = true;
                break;
            }
            Command::Append(text) => {
                writeln!(out, "{}", current).map_err(|e| e.to_string())?;
                writeln!(out, "{}", text).map_err(|e| e.to_string())?;
                printed = true;
            }
            Command::Insert(text) => {
                writeln!(out, "{}", text).map_err(|e| e.to_string())?;
                writeln!(out, "{}", current).map_err(|e| e.to_string())?;
                printed = true;
            }
            Command::Change(text) => {
                writeln!(out, "{}", text).map_err(|e| e.to_string())?;
                printed = true;
                deleted = true;
                break;
            }
        }
    }

    // Default print unless the line was deleted, -n is active, or an
    // explicit command already produced output.
    if !deleted && !flag_n && !printed {
        writeln!(out, "{}", current).map_err(|e| e.to_string())?;
    }

    Ok(())
}

register_command!(
    SED_CMD,
    "sed",
    "ne:f:",
    CommandFlags::BIN.bits(),
    sed_main
);