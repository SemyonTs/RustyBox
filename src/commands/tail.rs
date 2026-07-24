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
use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
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

    let mut exit_code: u8 = 0;
    let multiple = ctx.optargs.len() > 1 && !flag_q;

    let stdout = std::io::stdout();
    let mut writer = BufWriter::new(stdout.lock());

    if ctx.optargs.is_empty() {
        if multiple {
            writeln!(writer, "==> - <==").ok();
        } else if flag_v {
            writeln!(writer, "==> - <==").ok();
        }
        if let Err(e) = tail_file("-", lines, bytes, flag_f, &mut writer) {
            eprintln!("tail: {e}");
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

            if let Err(e) = tail_file(file, lines, bytes, flag_f, &mut writer) {
                eprintln!("tail: {e}");
                exit_code = 1;
            }
        }
    }

    writer.flush().ok();
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
    writer: &mut BufWriter<std::io::StdoutLock>,
) -> Result<(), String> {
    // Stdin path: use a ring buffer to keep only the last N lines in memory.
    if file == "-" {
        let stdin = std::io::stdin();
        let mut reader = stdin.lock();

        if bytes >= 0 {
            // Byte-count from stdin: read all, keep only the tail.
            let mut buf = Vec::new();
            reader
                .take(bytes as u64)
                .read_to_end(&mut buf)
                .map_err(|e| e.to_string())?;
            writer.write_all(&buf).map_err(|e| e.to_string())?;
            return Ok(());
        }

        // Line-count from stdin: ring buffer with only `lines` entries.
        let cap = if lines >= 0 {
            lines as usize
        } else {
            usize::MAX // +N: keep everything from line N onward.
        };
        let mut ring = VecDeque::with_capacity(if cap < 1024 { cap } else { 1024 });
        let mut line_num: i64 = 0;

        let mut line_buf = String::new();
        loop {
            line_buf.clear();
            let n = reader.read_line(&mut line_buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            line_num += 1;

            if lines >= 0 {
                // -n N: keep last N lines.
                if ring.len() == cap {
                    ring.pop_front();
                }
                // Store without trailing newline.
                let stored = if line_buf.ends_with('\n') {
                    line_buf[..line_buf.len() - 1].to_string()
                } else {
                    line_buf.clone()
                };
                ring.push_back(stored);
            } else {
                // +N: skip first N-1 lines, output the rest.
                if line_num >= lines {
                    writer
                        .write_all(line_buf.as_bytes())
                        .map_err(|e| e.to_string())?;
                }
            }
        }

        if lines >= 0 {
            for line in &ring {
                writeln!(writer, "{}", line).map_err(|e| e.to_string())?;
            }
        }
        return Ok(());
    }

    // File path.
    let mut f = File::open(file).map_err(|e| format!("'{}': {}", file, e))?;

    // Byte-count mode: seek backwards and read the remainder.
    if bytes >= 0 {
        let meta = f.metadata().map_err(|e| e.to_string())?;
        let size = meta.len() as i64;
        let start = (size - bytes).max(0);
        if start > 0 {
            f.seek(SeekFrom::Start(start as u64))
                .map_err(|e| e.to_string())?;
        }
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        writer.write_all(&buf).map_err(|e| e.to_string())?;

        if flag_f {
            follow(file, writer)?;
        }
        return Ok(());
    }

    // Line-count mode for files: ring buffer approach.
    let cap = if lines >= 0 {
        lines as usize
    } else {
        usize::MAX
    };
    let mut ring = VecDeque::with_capacity(if cap < 1024 { cap } else { 1024 });
    let mut line_num: i64 = 0;

    let reader = BufReader::new(f);
    let _line_buf = String::new();
    for line_result in reader.lines() {
        let line = line_result.map_err(|e| e.to_string())?;
        line_num += 1;

        if lines >= 0 {
            if ring.len() == cap {
                ring.pop_front();
            }
            ring.push_back(line);
        } else {
            if line_num >= lines {
                writeln!(writer, "{}", line).map_err(|e| e.to_string())?;
            }
        }
    }

    if lines >= 0 {
        for line in &ring {
            writeln!(writer, "{}", line).map_err(|e| e.to_string())?;
        }
    }

    if flag_f {
        follow(file, writer)?;
    }

    Ok(())
}

/// Follow a file, polling every 500 ms for new data and writing it to `out`.
///
/// This function never returns voluntarily; it is intended for `-f` mode.
fn follow(file: &str, out: &mut impl Write) -> Result<(), String> {
    let mut f = File::open(file).map_err(|e| format!("'{}': {}", file, e))?;
    f.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;

    let mut buf = [0u8; 4096];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n > 0 {
            out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            out.flush().map_err(|e| e.to_string())?;
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
