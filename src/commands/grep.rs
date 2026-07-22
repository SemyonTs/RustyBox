// =============================================================================
// grep — Search for lines matching a regular expression.
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
//   -E         Interpret patterns as extended regular expressions (default).
//   -F         Interpret patterns as fixed strings (literal match).
//   -i         Case-insensitive matching.
//   -v         Invert match: select non-matching lines.
//   -n         Prefix each output line with its 1-based line number.
//   -r, -R     Recursively search directories.
//   -c         Print only a count of matching lines per file.
//   -l         Print only the names of files containing at least one match.
//   -q         Quiet: suppress all normal output; exit immediately on first match.
//   -w         Match whole words only.
//   -x         Match whole lines only (anchored at both ends).
//   -e PATTERN Use PATTERN as the pattern (may be repeated).
//   -f FILE    Read patterns from FILE, one per line.
//   -h         Suppress the filename prefix in output.
//   -H         Force the filename prefix in output.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use regex::Regex;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};

/// Entry point for the `grep` builtin.
///
/// Exit codes:
///   0 — at least one match was found.
///   1 — no matches were found.
///   2 — an error occurred (syntax error in the pattern, etc.).
fn grep_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "EFivnrc(lq)wxe:f:") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("grep: {e}");
            return 2;
        }
    };

    // -E is recognised but is the default behaviour; the flag variable is
    // retained for compatibility with option-string parsing.
    let _flag_E = opts.count('E') > 0;
    let flag_F = opts.count('F') > 0;
    let flag_i = opts.count('i') > 0;
    let flag_v = opts.count('v') > 0;
    let flag_n = opts.count('n') > 0;
    let flag_r = opts.count('r') > 0 || opts.count('R') > 0;
    let flag_c = opts.count('c') > 0;
    let flag_l = opts.count('l') > 0;
    let flag_q = opts.count('q') > 0;
    let flag_w = opts.count('w') > 0;
    let flag_x = opts.count('x') > 0;
    let flag_h = opts.count('h') > 0;
    let flag_H = opts.count('H') > 0;

    // Collect patterns from -e and -f options.
    let mut patterns: Vec<String> = Vec::new();

    if let Some(e) = opts.get_str('e') {
        if !e.is_empty() {
            patterns.push(e.to_string());
        }
    }

    if let Some(f) = opts.get_str('f') {
        if let Ok(file) = File::open(f) {
            for line in BufReader::new(file).lines() {
                if let Ok(l) = line {
                    patterns.push(l);
                }
            }
        }
    }

    // If no -e/-f were given the first positional argument is the pattern.
    let mut args: Vec<String> = ctx.optargs.clone();
    if patterns.is_empty() {
        if args.is_empty() {
            eprintln!("grep: no pattern specified");
            return 2;
        }
        patterns.push(args.remove(0));
    }

    // Compile the patterns into regular expressions.
    let mut regexes = Vec::new();
    for p in &patterns {
        let re_str = if flag_F {
            regex::escape(p)
        } else {
            let mut s = p.clone();
            if flag_w {
                s = format!(r"\b{}\b", s);
            }
            if flag_x {
                s = format!("^{}$", s);
            }
            s
        };

        let re = if flag_i {
            Regex::new(&format!("(?i){}", re_str))
        } else {
            Regex::new(&re_str)
        };

        match re {
            Ok(r) => regexes.push(r),
            Err(e) => {
                eprintln!("grep: invalid regular expression '{}': {}", p, e);
                return 2;
            }
        }
    }

    let files: Vec<String> = if args.is_empty() {
        vec!["-".to_string()]
    } else {
        args
    };

    let multiple = files.len() > 1 || flag_r;
    let mut found_any = false;

    // Use a buffered stdout lock for all output.
    let stdout = std::io::stdout();
    let mut writer = BufWriter::new(stdout.lock());

    for file in &files {
        let mut file_list = vec![file.clone()];

        // Expand directories recursively when -r is active.
        if flag_r && file != "-" {
            file_list = list_files_recursive(file);
        }

        for f in &file_list {
            match grep_file(
                f,
                &regexes,
                flag_v,
                flag_n,
                flag_c,
                flag_l,
                flag_q,
                flag_h,
                flag_H,
                multiple,
                &mut writer,
            ) {
                Ok(found) => {
                    if found {
                        found_any = true;
                    }
                }
                Err(e) => {
                    if !flag_q {
                        eprintln!("grep: {e}");
                    }
                }
            }

            // Short-circuit: -q exits immediately on the first match.
            if flag_q && found_any {
                return 0;
            }
        }
    }

    // Flush writer (important!)
    writer.flush().ok();

    if flag_q {
        return if found_any { 0 } else { 1 };
    }

    if found_any {
        0
    } else {
        1
    }
}

/// Recursively collect regular files under `dir`.
///
/// Symlinks are not followed; only plain files are included in the result.
fn list_files_recursive(dir: &str) -> Vec<String> {
    let mut result = Vec::new();

    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return vec![dir.to_string()],
    };

    for item in rd {
        if let Ok(entry) = item {
            let path = entry.path();
            let meta = match std::fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if meta.is_dir() {
                result.extend(list_files_recursive(&path.to_string_lossy()));
            } else if meta.is_file() {
                result.push(path.to_string_lossy().into_owned());
            }
        }
    }

    result
}

/// Search a single file (or stdin when `file == "-"`) and print matches
/// according to the active flags.
///
/// Returns `Ok(true)` if at least one line matched.
fn grep_file(
    file: &str,
    regexes: &[Regex],
    flag_v: bool,
    flag_n: bool,
    flag_c: bool,
    flag_l: bool,
    flag_q: bool,
    flag_h: bool,
    flag_H: bool,
    multiple: bool,
    writer: &mut BufWriter<std::io::StdoutLock>,
) -> Result<bool, String> {
    let mut reader: Box<dyn BufRead> = if file == "-" {
        Box::new(std::io::stdin().lock())
    } else {
        let f = File::open(file).map_err(|e| format!("'{file}': {e}"))?;
        Box::new(BufReader::new(f))
    };

    let mut count = 0u64;
    let mut found = false;
    let show_name = (multiple && !flag_h) || flag_H;

    // Reusable line buffer – avoids allocating a new String for every line.
    let mut line_buf = String::with_capacity(256);
    let mut line_number = 0usize;

    loop {
        line_buf.clear();
        let n = reader
            .read_line(&mut line_buf)
            .map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        line_number += 1;

        // Remove trailing newline for accurate output if needed.
        // But for matching we can leave it; only when we print we may want to
        // strip it to avoid double newlines (since println! adds one).
        let line = if line_buf.ends_with('\n') {
            &line_buf[..line_buf.len() - 1]
        } else {
            &line_buf[..]
        };

        let matched = regexes.iter().any(|r| r.is_match(line));
        let is_match = if flag_v { !matched } else { matched };

        if is_match {
            found = true;
            count += 1;

            if flag_q {
                return Ok(true);
            }

            if flag_l {
                writeln!(writer, "{}", file).map_err(|e| e.to_string())?;
                return Ok(true);
            }

            if flag_c {
                continue;
            }

            // Build prefix efficiently without allocating a new String
            // for the common case where no prefix is needed.
            if show_name || flag_n {
                if show_name {
                    write!(writer, "{}:", file).map_err(|e| e.to_string())?;
                }
                if flag_n {
                    write!(writer, "{}:", line_number).map_err(|e| e.to_string())?;
                }
            }
            writeln!(writer, "{}", line).map_err(|e| e.to_string())?;
        }
    }

    if flag_c {
        if !flag_q {
            if show_name {
                writeln!(writer, "{}:{}", file, count).map_err(|e| e.to_string())?;
            } else {
                writeln!(writer, "{}", count).map_err(|e| e.to_string())?;
            }
        }
        return Ok(count > 0);
    }

    Ok(found)
}

register_command!(
    GREP_CMD,
    "grep",
    "EFivnrc(lq)wxe:f:",
    CommandFlags::BIN.bits(),
    grep_main
);