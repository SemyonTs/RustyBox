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
use std::io::{BufRead, BufReader, BufWriter, Write};

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
/// supplied) the first positional argument.
fn sed_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "ne:f:") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("sed: {e}");
            return 1;
        }
    };

    let flag_n = opts.count('n') > 0;

    // Accumulate script strings without cloning the entire optargs.
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

    // Determine file arguments: if no -e/-f were given, the first positional
    // argument is the script. The rest are files (or "-" if none).
    let files: &[String] = if scripts.is_empty() {
        if ctx.optargs.is_empty() {
            eprintln!("sed: no script specified");
            return 1;
        }
        scripts.push(ctx.optargs[0].clone());
        &ctx.optargs[1..]
    } else {
        &ctx.optargs[..]
    };

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

    let stdout = std::io::stdout();
    let mut writer = BufWriter::new(stdout.lock());
    let mut exit_code: u8 = 0;

    // Reusable line buffer for processing.
    let mut line_buf = String::with_capacity(256);

    if files.is_empty() {
        // stdin only.
        exit_code = process_reader(
            &mut std::io::stdin().lock(),
            &cmds,
            flag_n,
            &mut writer,
            &mut line_buf,
        );
    } else {
        for file in files {
            if file == "-" {
                exit_code |= process_reader(
                    &mut std::io::stdin().lock(),
                    &cmds,
                    flag_n,
                    &mut writer,
                    &mut line_buf,
                );
            } else {
                match File::open(file) {
                    Ok(f) => {
                        let mut reader = BufReader::new(f);
                        exit_code |=
                            process_reader(&mut reader, &cmds, flag_n, &mut writer, &mut line_buf);
                    }
                    Err(e) => {
                        eprintln!("sed: '{}': {}", file, e);
                        exit_code = 1;
                    }
                }
            }
        }
    }

    writer.flush().ok();
    exit_code
}

/// Process all lines from a buffered reader through the compiled commands.
fn process_reader(
    reader: &mut dyn BufRead,
    cmds: &[Command],
    flag_n: bool,
    writer: &mut BufWriter<std::io::StdoutLock>,
    line_buf: &mut String,
) -> u8 {
    loop {
        line_buf.clear();
        match reader.read_line(line_buf) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(_) => return 1,
        }

        // Remove trailing newline for consistent processing, then add it back on output.
        let line = if line_buf.ends_with('\n') {
            &line_buf[..line_buf.len() - 1]
        } else {
            &line_buf[..]
        };

        if let Err(e) = process_line(line, cmds, flag_n, writer) {
            eprintln!("sed: {e}");
            return 1;
        }
    }
    0
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
/// to `writer`.
fn process_line<W: Write>(
    line: &str,
    cmds: &[Command],
    flag_n: bool,
    writer: &mut W,
) -> Result<(), String> {
    let mut current: std::borrow::Cow<'_, str> = std::borrow::Cow::Borrowed(line);
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
                    current = std::borrow::Cow::Owned(
                        pattern
                            .replace_all(current.as_ref(), repl.as_str())
                            .to_string(),
                    );
                } else {
                    current = std::borrow::Cow::Owned(
                        pattern.replace(current.as_ref(), repl.as_str()).to_string(),
                    );
                }
            }
            Command::Print => {
                writeln!(writer, "{}", current).map_err(|e| e.to_string())?;
                printed = true;
            }
            Command::Delete => {
                deleted = true;
                break;
            }
            Command::Append(text) => {
                writeln!(writer, "{}", current).map_err(|e| e.to_string())?;
                writeln!(writer, "{}", text).map_err(|e| e.to_string())?;
                printed = true;
            }
            Command::Insert(text) => {
                writeln!(writer, "{}", text).map_err(|e| e.to_string())?;
                writeln!(writer, "{}", current).map_err(|e| e.to_string())?;
                printed = true;
            }
            Command::Change(text) => {
                writeln!(writer, "{}", text).map_err(|e| e.to_string())?;
                printed = true;
                deleted = true;
                break;
            }
        }
    }

    // Default print unless the line was deleted, -n is active, or an
    // explicit command already produced output.
    if !deleted && !flag_n && !printed {
        writeln!(writer, "{}", current).map_err(|e| e.to_string())?;
    }

    Ok(())
}

register_command!(SED_CMD, "sed", "ne:f:", CommandFlags::BIN.bits(), sed_main);
