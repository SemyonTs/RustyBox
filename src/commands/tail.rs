// =============================================================================
// tail — Output the last part of files.
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
//   -n NUM   Output the last NUM lines (default: 10).  +NUM starts from
//            line NUM.
//   -c NUM   Output the last NUM bytes.
//   -f       Follow: append data as the file grows.
//   -q       Suppress the filename header in multi-file output.
//   -v       Always print the filename header.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::thread;
use std::time::Duration;

/// Entry point for the `tail` builtin.
///
/// When no file arguments are given stdin is read.
fn tail_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "n:>0c:>0fqv") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("tail: {e}");
            return 1;
        }
    };

    let lines = opts.get_int('n').unwrap_or(10);
    let bytes = opts.get_int('c').unwrap_or(-1);
    let flag_f = opts.count('f') > 0;
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

        if let Err(e) = tail_file(file, lines, bytes, flag_f) {
            eprintln!("tail: {e}");
            exit_code = 1;
        }
    }

    exit_code
}

/// Print the tail of a single file (or stdin when `file == "-"`).
///
/// When `-f` is active the function blocks and continues to read appended
/// data indefinitely.
fn tail_file(
    file: &str,
    lines: i64,
    bytes: i64,
    flag_f: bool,
) -> Result<(), String> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    // Stdin path: read all lines into memory, then emit the tail.
    if file == "-" {
        let stdin = std::io::stdin();
        let reader = stdin.lock();
        let all: Vec<String> =
            reader.lines().map(|l| l.unwrap_or_default()).collect();

        let start = if lines >= 0 {
            all.len().saturating_sub(lines as usize)
        } else {
            // +N: start at line N (1-based).
            (lines.unsigned_abs() as usize).saturating_sub(1)
        };

        for line in &all[start..] {
            writeln!(out, "{}", line).map_err(|e| e.to_string())?;
        }
        return Ok(());
    }

    let mut f = File::open(file).map_err(|e| format!("'{}': {}", file, e))?;

    // Byte-count mode: seek backwards and read the remainder.
    if bytes >= 0 {
        let meta = f.metadata().map_err(|e| e.to_string())?;
        let size = meta.len() as i64;
        let start = size - bytes;
        if start > 0 {
            f.seek(SeekFrom::Start(start as u64))
                .map_err(|e| e.to_string())?;
        }
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        out.write_all(&buf).map_err(|e| e.to_string())?;

        if flag_f {
            follow(file, &mut out)?;
        }
        return Ok(());
    }

    // Line-count mode: read all lines, then emit the tail.
    let reader = BufReader::new(f);
    let all: Vec<String> =
        reader.lines().map(|l| l.unwrap_or_default()).collect();

    let start = if lines >= 0 {
        all.len().saturating_sub(lines as usize)
    } else {
        (lines.unsigned_abs() as usize).saturating_sub(1)
    };

    for line in &all[start..] {
        writeln!(out, "{}", line).map_err(|e| e.to_string())?;
    }

    if flag_f {
        follow(file, &mut out)?;
    }

    Ok(())
}

/// Follow a file, polling every 500 ms for new data and writing it to `out`.
///
/// This function never returns voluntarily; it is intended for `-f` mode.
fn follow(file: &str, out: &mut impl Write) -> Result<(), String> {
    let mut f =
        File::open(file).map_err(|e| format!("'{}': {}", file, e))?;
    f.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;

    let mut buf = [0u8; 4096];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n > 0 {
            out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            out.flush().ok();
        }
        thread::sleep(Duration::from_millis(500));
    }
}

register_command!(
    TAIL_CMD,
    "tail",
    "n:>0c:>0fqv",
    CommandFlags::BIN.bits(),
    tail_main
);