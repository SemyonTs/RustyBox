// =============================================================================
// head — Output the first part of files.
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
//   -n NUM   Print the first NUM lines (default: 10).
//   -c NUM   Print the first NUM bytes.
//   -q       Suppress the filename header in multi-file output.
//   -v       Always print the filename header.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};

#[derive(Clone, Copy)]
enum HeadMode {
    Lines(i64),
    Bytes(i64),
}

/// Entry point for the `head` builtin.
///
/// The option string `"n:>0c:>0qv"` enforces that -n and -c values are
/// positive integers in the parser, but we also handle negative values
/// manually to support GNU head's "all but last N" extension.
fn head_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "n:c:qv") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("head: {e}");
            return 1;
        }
    };

    let lines_opt = opts.get_int('n').map(|x| x as i64);
    let bytes_opt = opts.get_int('c').map(|x| x as i64);

    // Validate that if -n or -c are provided, they have valid integer arguments.
    if opts.count('n') > 0 && lines_opt.is_none() {
        eprintln!("head: invalid number of lines");
        return 1;
    }
    if opts.count('c') > 0 && bytes_opt.is_none() {
        eprintln!("head: invalid number of bytes");
        return 1;
    }

    // Determine which mode wins if both are specified.
    // Per GNU head behavior, the last specified option between -n and -c wins.
    let mut last_mode = 'n';
    for arg in std::env::args() {
        if arg == "-n" || arg.starts_with("-n") {
            last_mode = 'n';
        } else if arg == "-c" || arg.starts_with("-c") {
            last_mode = 'c';
        }
    }

    let mode = match (lines_opt, bytes_opt) {
        (Some(_), Some(_)) => {
            if last_mode == 'c' {
                HeadMode::Bytes(bytes_opt.unwrap())
            } else {
                HeadMode::Lines(lines_opt.unwrap())
            }
        }
        (Some(l), None) => HeadMode::Lines(l),
        (None, Some(b)) => HeadMode::Bytes(b),
        (None, None) => HeadMode::Lines(10),
    };

    let flag_q = opts.count('q') > 0;
    let flag_v = opts.count('v') > 0;

    let mut exit_code: u8 = 0;
    let print_header = (ctx.optargs.len() > 1 || flag_v) && !flag_q;

    let stdout = std::io::stdout();
    let mut writer = BufWriter::new(stdout.lock());

    if ctx.optargs.is_empty() {
        if print_header {
            writeln!(writer, "==> - <==").ok();
        }
        if let Err(e) = head_file("-", mode, &mut writer) {
            eprintln!("head: {e}");
            exit_code = 1;
        }
    } else {
        for (i, file) in ctx.optargs.iter().enumerate() {
            if print_header {
                if i > 0 {
                    writeln!(writer).ok();
                }
                writeln!(writer, "==> {} <==", file).ok();
            }

            if let Err(e) = head_file(file, mode, &mut writer) {
                eprintln!("head: {e}");
                exit_code = 1;
            }
        }
    }

    writer.flush().ok();
    exit_code
}

/// Read the head of a single file (or stdin when `file == "-"`) and write
/// the requested number of lines or bytes to stdout.
fn head_file(file: &str, mode: HeadMode, writer: &mut impl Write) -> Result<(), String> {
    match mode {
        HeadMode::Bytes(b) if b >= 0 => {
            let mut reader: Box<dyn Read> = if file == "-" {
                Box::new(std::io::stdin())
            } else {
                Box::new(File::open(file).map_err(|e| format!("'{file}': {e}"))?)
            };
            std::io::copy(&mut reader.take(b as u64), writer).map_err(|e| e.to_string())?;
        }
        HeadMode::Bytes(b) => {
            // b is negative: print all but the last (-b) bytes.
            let drop_bytes = (-b) as usize;
            let data = if file == "-" {
                let mut data = Vec::new();
                std::io::stdin()
                    .read_to_end(&mut data)
                    .map_err(|e| e.to_string())?;
                data
            } else {
                std::fs::read(file).map_err(|e| format!("'{file}': {e}"))?
            };
            if data.len() > drop_bytes {
                writer
                    .write_all(&data[..data.len() - drop_bytes])
                    .map_err(|e| e.to_string())?;
            }
        }
        HeadMode::Lines(l) if l >= 0 => {
            let mut line = String::new();
            let mut reader: Box<dyn BufRead> = if file == "-" {
                let stdin = std::io::stdin();
                Box::new(stdin.lock())
            } else {
                let f = File::open(file).map_err(|e| format!("'{file}': {e}"))?;
                Box::new(BufReader::new(f))
            };
            for _ in 0..l {
                line.clear();
                let n = reader.read_line(&mut line).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                writer
                    .write_all(line.as_bytes())
                    .map_err(|e| e.to_string())?;
            }
        }
        HeadMode::Lines(l) => {
            // l is negative: print all but the last (-l) lines.
            let drop_lines = (-l) as usize;
            let mut all_lines = Vec::new();
            let mut reader: Box<dyn BufRead> = if file == "-" {
                let stdin = std::io::stdin();
                Box::new(stdin.lock())
            } else {
                let f = File::open(file).map_err(|e| format!("'{file}': {e}"))?;
                Box::new(BufReader::new(f))
            };
            let mut line = String::new();
            loop {
                line.clear();
                let n = reader.read_line(&mut line).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                all_lines.push(line.clone());
            }
            let print_until = if all_lines.len() > drop_lines {
                all_lines.len() - drop_lines
            } else {
                0
            };
            for i in 0..print_until {
                writer
                    .write_all(all_lines[i].as_bytes())
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

register_command!(
    HEAD_CMD,
    "head",
    "n:>0c:>0qv",
    CommandFlags::BIN.bits(),
    head_main
);
