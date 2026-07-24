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

    let mut exit_code: u8 = 0;
    let multiple = ctx.optargs.len() > 1 && !flag_q;

    // Reusable read buffer for byte-count mode — allocated once.
    let mut byte_buf: Option<Vec<u8>> = if bytes >= 0 {
        Some(vec![0u8; bytes as usize])
    } else {
        None
    };

    let stdout = std::io::stdout();
    let mut writer = BufWriter::new(stdout.lock());

    if ctx.optargs.is_empty() {
        if multiple {
            writeln!(writer, "==> - <==").ok();
        } else if flag_v {
            writeln!(writer, "==> - <==").ok();
        }
        if let Err(e) = head_file("-", lines, bytes, &mut byte_buf, &mut writer) {
            eprintln!("head: {e}");
            exit_code = 1;
        }
    } else {
        for (i, file) in ctx.optargs.iter().enumerate() {
            if multiple {
                if i > 0 {
                    writeln!(writer).ok();
                }
                writeln!(writer, "==> {} <==", file).ok();
            } else if flag_v && file != "-" {
                writeln!(writer, "==> {} <==", file).ok();
            }

            if let Err(e) = head_file(file, lines, bytes, &mut byte_buf, &mut writer) {
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
///
/// `byte_buf` is `Some(buf)` in byte-count mode and reused across calls;
/// `None` in line-count mode.
fn head_file(
    file: &str,
    lines: usize,
    bytes: i64,
    byte_buf: &mut Option<Vec<u8>>,
    writer: &mut impl Write,
) -> Result<(), String> {
    // Byte-count mode: read up to `bytes` bytes, then stop.
    if bytes >= 0 {
        let buf = byte_buf.as_mut().unwrap();
        let n = if file == "-" {
            let mut stdin = std::io::stdin();
            stdin.read(buf).map_err(|e| e.to_string())?
        } else {
            let mut f = File::open(file).map_err(|e| format!("'{file}': {e}"))?;
            f.read(buf).map_err(|e| format!("'{file}': {e}"))?
        };
        writer.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        return Ok(());
    }

    // Line-count mode: read up to `lines` lines.
    let mut reader: Box<dyn BufRead> = if file == "-" {
        Box::new(std::io::stdin().lock())
    } else {
        let f = File::open(file).map_err(|e| format!("'{file}': {e}"))?;
        Box::new(BufReader::new(f))
    };

    for _ in 0..lines {
        let mut line = String::new();
        let n = reader.read_line(&mut line).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        writer
            .write_all(line.as_bytes())
            .map_err(|e| e.to_string())?;
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
