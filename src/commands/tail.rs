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
    let opts = match crate::args::parse(ctx, "n:c:fqv") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("tail: {e}");
            return 1;
        }
    };

    // Get the raw value of -n to detect +N syntax
    let n_raw = opts.get_str('n').unwrap_or("");
    let mut lines = 10; // default
    let mut from_line = false;

    if !n_raw.is_empty() {
        if n_raw.starts_with('+') {
            // +N mode: start from line N
            if let Ok(n) = n_raw[1..].parse::<i64>() {
                lines = n;
                from_line = true;
            }
        } else {
            // -n N mode: last N lines
            if let Ok(n) = n_raw.parse::<i64>() {
                lines = n;
            }
        }
    }

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
        if let Err(e) = tail_file("-", lines, from_line, bytes, flag_f, &mut writer) {
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

            if let Err(e) = tail_file(file, lines, from_line, bytes, flag_f, &mut writer) {
                eprintln!("tail: {e}");
                exit_code = 1;
            }
        }
    }

    // Final flush in case follow mode never happened or for non-follow cases.
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
    from_line: bool,
    bytes: i64,
    flag_f: bool,
    writer: &mut BufWriter<std::io::StdoutLock>,
) -> Result<(), String> {
    // Stdin path: use a ring buffer to keep only the last N lines/bytes.
    if file == "-" {
        let stdin = std::io::stdin();
        let mut reader = stdin.lock();

        if bytes >= 0 {
            // Byte-count from stdin: keep only the last <bytes> bytes.
            let cap = bytes as usize;
            if cap == 0 {
                return Ok(());
            }
            let mut ring: VecDeque<u8> = VecDeque::with_capacity(cap);
            let mut buf = [0u8; 4096];
            loop {
                let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                for &b in &buf[..n] {
                    if ring.len() == cap {
                        ring.pop_front();
                    }
                    ring.push_back(b);
                }
            }
            // Output the buffered tail
            let bytes_to_write: Vec<u8> = ring.iter().copied().collect();
            writer
                .write_all(&bytes_to_write)
                .map_err(|e| e.to_string())?;

            if flag_f {
                // Flush any buffered tail output before entering follow mode.
                writer.flush().map_err(|e| e.to_string())?;
                // Follow stdin: keep reading and writing new data
                loop {
                    let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
                    if n == 0 {
                        break; // EOF on follow means the input stream ended
                    }
                    writer.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                    writer.flush().map_err(|e| e.to_string())?;
                }
            }
            return Ok(());
        }

        if from_line {
            // +N: skip first N-1 lines, output the rest
            let mut line_num = 0;
            let mut line_buf = String::new();
            loop {
                line_buf.clear();
                let n = reader.read_line(&mut line_buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                line_num += 1;
                if line_num >= lines {
                    writer
                        .write_all(line_buf.as_bytes())
                        .map_err(|e| e.to_string())?;
                }
            }
            if flag_f {
                // Flush before follow so the test (or any pipe consumer) sees the initial output.
                writer.flush().map_err(|e| e.to_string())?;
                // Follow: keep reading lines and output immediately
                let mut line_buf = String::new();
                loop {
                    line_buf.clear();
                    let n = reader.read_line(&mut line_buf).map_err(|e| e.to_string())?;
                    if n == 0 {
                        break;
                    }
                    writer
                        .write_all(line_buf.as_bytes())
                        .map_err(|e| e.to_string())?;
                    writer.flush().map_err(|e| e.to_string())?;
                }
            }
            return Ok(());
        }

        // -n N: keep last N lines
        if lines <= 0 {
            return Ok(()); // -n 0 or negative: nothing
        }

        let cap = lines as usize;
        let mut ring = VecDeque::with_capacity(if cap < 1024 { cap } else { 1024 });
        let mut line_buf = String::new();

        loop {
            line_buf.clear();
            let n = reader.read_line(&mut line_buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
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
        }

        for line in &ring {
            writeln!(writer, "{}", line).map_err(|e| e.to_string())?;
        }

        if flag_f {
            // Flush the initial ring-buffer output before entering follow mode.
            writer.flush().map_err(|e| e.to_string())?;
            // Follow: continue reading lines and output them
            let mut line_buf = String::new();
            loop {
                line_buf.clear();
                let n = reader.read_line(&mut line_buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                writer
                    .write_all(line_buf.as_bytes())
                    .map_err(|e| e.to_string())?;
                writer.flush().map_err(|e| e.to_string())?;
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
        // Flush before follow to ensure initial bytes are visible immediately.
        writer.flush().map_err(|e| e.to_string())?;

        if flag_f {
            // File pointer is already at the end; do not rewind.
            follow_file(&mut f, writer)?;
        }
        return Ok(());
    }

    // Line-count mode for files.
    let mut ring = VecDeque::new();
    let mut line_num = 0;

    // Use BufReader directly on the file
    let reader = BufReader::new(&mut f);

    if from_line {
        // +N: start from line N
        for line_result in reader.lines() {
            let line = line_result.map_err(|e| e.to_string())?;
            line_num += 1;
            if line_num >= lines {
                writeln!(writer, "{}", line).map_err(|e| e.to_string())?;
            }
        }
    } else {
        // -n N: last N lines
        if lines <= 0 {
            // -n 0 or negative: nothing
            if flag_f {
                // Still need to flush before follow (even if no output)
                writer.flush().map_err(|e| e.to_string())?;
                follow_file(&mut f, writer)?;
            }
            return Ok(());
        }

        for line_result in reader.lines() {
            let line = line_result.map_err(|e| e.to_string())?;
            if ring.len() == lines as usize {
                ring.pop_front();
            }
            ring.push_back(line);
        }

        for line in &ring {
            writeln!(writer, "{}", line).map_err(|e| e.to_string())?;
        }
    }

    // Flush any buffered tail output before entering follow mode.
    // This is critical for -f to work correctly when piped: the initial
    // "tail" lines must be flushed so they are visible to downstream
    // processes (including tests) before we start waiting for new data.
    writer.flush().map_err(|e| e.to_string())?;

    // If -f is specified, enter follow mode.
    // The file handle is already at the end because we've read everything;
    // no extra seek is performed to avoid missing data.
    if flag_f {
        follow_file(&mut f, writer)?;
    }

    Ok(())
}

/// Follow a file using an existing file handle, reading new data as it's appended.
///
/// The caller must ensure that the file pointer is already at the current end
/// of the file.  This function never returns voluntarily; it is intended for
/// `-f` mode.
fn follow_file(f: &mut File, out: &mut impl Write) -> Result<(), String> {
    // Note: we no longer seek to the end here because the caller has
    // positioned the file pointer exactly where we want to continue reading.
    // Seeking again would risk skipping data written between the last read
    // and the seek call.

    let mut buf = [0u8; 4096];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n > 0 {
            out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            out.flush().map_err(|e| e.to_string())?;
        }
        // Poll frequently for tests
        thread::sleep(Duration::from_millis(50));
    }
}

register_command!(
    TAIL_CMD,
    "tail",
    "n:c:fqv",
    CommandFlags::BIN.bits(),
    tail_main
);
