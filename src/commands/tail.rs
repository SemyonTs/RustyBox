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
//            line NUM. -NUM is equivalent to NUM.
//   -c NUM   Output the last NUM bytes. +NUM starts from byte NUM.
//            -NUM is equivalent to NUM.
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

/// Parse tail's numeric arguments according to POSIX.
/// Returns (from_start, count).
/// "+5" -> (true, 5)
/// "5"  -> (false, 5)
/// "-5" -> (false, 5)
fn parse_tail_count(s: &str) -> Result<(bool, u64), String> {
    if let Some(stripped) = s.strip_prefix('+') {
        let n: u64 = stripped
            .parse()
            .map_err(|_| format!("invalid number: '{}'", s))?;
        Ok((true, n))
    } else {
        let s = s.strip_prefix('-').unwrap_or(s);
        let n: u64 = s.parse().map_err(|_| format!("invalid number: '{}'", s))?;
        Ok((false, n))
    }
}

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

    // Default: last 10 lines
    let mut lines: u64 = 10;
    let mut from_line = false;
    let mut use_bytes = false;
    let mut bytes: u64 = 0;

    if let Some(n_raw) = opts.get_str('n') {
        match parse_tail_count(n_raw) {
            Ok((from, count)) => {
                from_line = from;
                lines = count;
            }
            Err(e) => {
                eprintln!("tail: {}", e);
                return 1;
            }
        }
    }

    if let Some(c_raw) = opts.get_str('c') {
        match parse_tail_count(c_raw) {
            Ok((from, count)) => {
                use_bytes = true;
                from_line = from;
                bytes = count;
            }
            Err(e) => {
                eprintln!("tail: {}", e);
                return 1;
            }
        }
    }

    let flag_f = opts.count('f') > 0;
    let flag_q = opts.count('q') > 0;
    let flag_v = opts.count('v') > 0;

    let mut exit_code: u8 = 0;
    let multiple = ctx.optargs.len() > 1 && !flag_q;

    let stdout = std::io::stdout();
    let mut writer = BufWriter::new(stdout.lock());

    if ctx.optargs.is_empty() {
        if multiple || flag_v {
            writeln!(writer, "==> - <==").ok();
        }
        if let Err(e) = tail_file("-", lines, from_line, use_bytes, bytes, flag_f, &mut writer) {
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

            if let Err(e) = tail_file(
                file,
                lines,
                from_line,
                use_bytes,
                bytes,
                flag_f,
                &mut writer,
            ) {
                eprintln!("tail: {e}");
                exit_code = 1;
            }
        }
    }

    writer.flush().ok();
    exit_code
}

/// Print the tail of a single file (or stdin when `file == "-"`).
fn tail_file(
    file: &str,
    lines: u64,
    from_line: bool,
    use_bytes: bool,
    bytes: u64,
    flag_f: bool,
    writer: &mut BufWriter<std::io::StdoutLock>,
) -> Result<(), String> {
    // Stdin path
    if file == "-" {
        let stdin = std::io::stdin();
        let mut reader = stdin.lock();

        if use_bytes {
            if bytes == 0 {
                return Ok(());
            }
            if from_line {
                // +N bytes from stdin: skip N-1 bytes
                let skip = (bytes - 1) as usize;
                let mut skipped = 0;
                let mut buf = [0u8; 4096];
                while skipped < skip {
                    let to_read = (skip - skipped).min(buf.len());
                    let n = reader
                        .read(&mut buf[..to_read])
                        .map_err(|e| e.to_string())?;
                    if n == 0 {
                        break;
                    }
                    skipped += n;
                }
                // Copy remainder
                std::io::copy(&mut reader, writer).map_err(|e| e.to_string())?;
            } else {
                // Last N bytes from stdin
                let cap = bytes as usize;
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
                let bytes_to_write: Vec<u8> = ring.iter().copied().collect();
                writer
                    .write_all(&bytes_to_write)
                    .map_err(|e| e.to_string())?;
            }

            if flag_f {
                writer.flush().map_err(|e| e.to_string())?;
                let mut buf = [0u8; 4096];
                loop {
                    let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
                    if n == 0 {
                        break;
                    }
                    writer.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                    writer.flush().map_err(|e| e.to_string())?;
                }
            }
            return Ok(());
        }

        // Line mode for stdin
        if from_line {
            let mut line_num: u64 = 0;
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
        } else {
            if lines == 0 {
                return Ok(());
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
        }

        if flag_f {
            writer.flush().map_err(|e| e.to_string())?;
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

    // File path
    let mut f = File::open(file).map_err(|e| format!("'{}': {}", file, e))?;

    if use_bytes {
        if bytes == 0 {
            return Ok(());
        }
        if from_line {
            // +N bytes: seek to offset N-1
            let start = (bytes - 1) as u64;
            let meta = f.metadata().map_err(|e| e.to_string())?;
            if start < meta.len() {
                f.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
            } else {
                // Start beyond EOF: nothing to output initially
                f.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
            }
        } else {
            // Last N bytes
            let meta = f.metadata().map_err(|e| e.to_string())?;
            let size = meta.len();
            let start = size.saturating_sub(bytes);
            f.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
        }

        let mut buf = Vec::new();
        f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        writer.write_all(&buf).map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())?;

        if flag_f {
            follow_file(&mut f, writer)?;
        }
        return Ok(());
    }

    // Line-count mode for files
    let reader = BufReader::new(&mut f);

    if from_line {
        let mut line_num: u64 = 0;
        for line_result in reader.lines() {
            let line = line_result.map_err(|e| e.to_string())?;
            line_num += 1;
            if line_num >= lines {
                writeln!(writer, "{}", line).map_err(|e| e.to_string())?;
            }
        }
    } else {
        if lines == 0 {
            if flag_f {
                writer.flush().map_err(|e| e.to_string())?;
                follow_file(&mut f, writer)?;
            }
            return Ok(());
        }

        let mut ring = VecDeque::new();
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

    writer.flush().map_err(|e| e.to_string())?;

    if flag_f {
        follow_file(&mut f, writer)?;
    }

    Ok(())
}

/// Follow a file using an existing file handle, reading new data as it's appended.
fn follow_file(f: &mut File, out: &mut impl Write) -> Result<(), String> {
    let mut buf = [0u8; 4096];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n > 0 {
            out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            out.flush().map_err(|e| e.to_string())?;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

register_command!(
    TAIL_CMD,
    "tail",
    "n:c:fqv",
    CommandFlags::BIN.bits(),
    tail_main,
    description = "Output the last part of files",
    help = "\
OPTIONS:
-n NUM   Output the last NUM lines (default: 10).  +NUM starts from
         line NUM. -NUM is equivalent to NUM.
-c NUM   Output the last NUM bytes. +NUM starts from byte NUM.
         -NUM is equivalent to NUM.
-f       Follow: append data as the file grows.
-q       Suppress the filename header in multi-file output.
-v       Always print the filename header."
);
