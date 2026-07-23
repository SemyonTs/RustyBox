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
use std::io::{BufRead, BufReader, Read, Write};

/// Entry point for the `head` builtin.
///
/// The option string `"n:>0c:>0qv"` enforces that -n and -c values are
/// positive integers.
fn head_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "n:>0c:>0qv") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("head: {e}");
            return 1;
        }
    };

    let lines = opts.get_int('n').unwrap_or(10) as usize;
    let bytes = opts.get_int('c').unwrap_or(-1);
    let flag_q = opts.count('q') > 0;
    let flag_v = opts.count('v') > 0;

    let args: Vec<String> = ctx.optargs.clone();
    let files: Vec<String> = if args.is_empty() {
        vec!["-".to_string()]
    } else {
        args
    };

    let mut exit_code: u8 = 0;
    let multiple = files.len() > 1 && !flag_q;

    for (i, file) in files.iter().enumerate() {
        if multiple {
            if i > 0 {
                println!();
            }
            println!("==> {} <==", file);
        } else if flag_v && file != "-" {
            println!("==> {} <==", file);
        }

        if let Err(e) = head_file(file, lines, bytes) {
            eprintln!("head: {e}");
            exit_code = 1;
        }
    }

    exit_code
}

/// Read the head of a single file (or stdin when `file == "-"`) and write
/// the requested number of lines or bytes to stdout.
fn head_file(file: &str, lines: usize, bytes: i64) -> Result<(), String> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    // Byte-count mode: read exactly `bytes` bytes, then stop.
    if bytes >= 0 {
        let mut buf = vec![0u8; bytes as usize];
        let n = if file == "-" {
            let mut stdin = std::io::stdin();
            stdin.read(&mut buf).map_err(|e| e.to_string())?
        } else {
            let mut f = File::open(file).map_err(|e| format!("'{file}': {e}"))?;
            f.read(&mut buf).map_err(|e| format!("'{file}': {e}"))?
        };
        out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        return Ok(());
    }

    // Line-count mode: read up to `lines` lines.
    if file == "-" {
        let stdin = std::io::stdin();
        let reader = stdin.lock();
        for (i, line) in reader.lines().enumerate() {
            if i >= lines {
                break;
            }
            let line = line.map_err(|e| e.to_string())?;
            writeln!(out, "{}", line).map_err(|e| e.to_string())?;
        }
    } else {
        let f = File::open(file).map_err(|e| format!("'{file}': {e}"))?;
        let reader = BufReader::new(f);
        for (i, line) in reader.lines().enumerate() {
            if i >= lines {
                break;
            }
            let line = line.map_err(|e| e.to_string())?;
            writeln!(out, "{}", line).map_err(|e| e.to_string())?;
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
