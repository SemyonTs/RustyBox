// =============================================================================
// cut — Extract selected parts of each line of a file.
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Implementation inspired by Toybox (https://landley.net/toybox/)
// Toybox is copyrighted by Rob Landley, see NOTICE file for license details.
//
// Supported options (mutually exclusive selection modes):
//   -b LIST   Select bytes.
//   -c LIST   Select characters.
//   -f LIST   Select fields (delimited by -d, default TAB).
//   -d DELIM  Field delimiter (used with -f).
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};

/// Entry point for the `cut` builtin.
///
/// Exactly one selection mode must be specified via -b, -c, or -f.
/// The option string `b:c:f:d:` defines three mutually exclusive flag groups
/// plus a delimiter argument for field mode.
fn cut_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "b:c:f:d:") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("cut: {e}");
            return 1;
        }
    };

    // Borrow directly — no unnecessary to_string() for option values that are
    // only used as &str.
    let bytes_raw = opts.get_str('b').unwrap_or("");
    let chars_raw = opts.get_str('c').unwrap_or("");
    let fields_raw = opts.get_str('f').unwrap_or("");
    let delim_str_raw = opts.get_str('d').unwrap_or("\t");

    let bytes_ranges = if !bytes_raw.is_empty() {
        match parse_list(bytes_raw) {
            Ok(r) => Some(r),
            Err(e) => {
                eprintln!("cut: {e}");
                return 1;
            }
        }
    } else {
        None
    };

    let chars_ranges = if !chars_raw.is_empty() {
        match parse_list(chars_raw) {
            Ok(r) => Some(r),
            Err(e) => {
                eprintln!("cut: {e}");
                return 1;
            }
        }
    } else {
        None
    };

    let fields_ranges = if !fields_raw.is_empty() {
        match parse_list(fields_raw) {
            Ok(r) => Some(r),
            Err(e) => {
                eprintln!("cut: {e}");
                return 1;
            }
        }
    } else {
        None
    };

    let mut modes = 0;
    if bytes_ranges.is_some() {
        modes += 1;
    }
    if chars_ranges.is_some() {
        modes += 1;
    }
    if fields_ranges.is_some() {
        modes += 1;
    }

    if modes == 0 {
        eprintln!("cut: need to specify -b, -c or -f");
        return 1;
    }
    if modes > 1 {
        eprintln!("cut: only one of -b, -c or -f may be specified");
        return 1;
    }

    let mode = if let Some(r) = bytes_ranges {
        Mode::Bytes(canonical_ranges(&r))
    } else if let Some(r) = chars_ranges {
        Mode::Chars(canonical_ranges(&r))
    } else {
        let r = fields_ranges.unwrap();
        let delim_char = delim_str_raw.chars().next().unwrap_or('\t');
        Mode::Fields {
            ranges: canonical_ranges(&r),
            delim_char,
        }
    };

    // No cloning of the entire optargs vector — just borrow or use a single
    // allocation for the fallback "-".
    let mut exit_code: u8 = 0;

    if ctx.optargs.is_empty() {
        if let Err(e) = cut_file("-", &mode) {
            eprintln!("cut: {e}");
            exit_code = 1;
        }
    } else {
        for file in &ctx.optargs {
            if let Err(e) = cut_file(file, &mode) {
                eprintln!("cut: {e}");
                exit_code = 1;
            }
        }
    }

    exit_code
}

/// Selection mode determined by the command-line flags.
enum Mode {
    /// Select byte positions (1‑based).
    Bytes(Vec<(usize, usize)>),
    /// Select character positions (1‑based).
    Chars(Vec<(usize, usize)>),
    /// Select fields separated by the given delimiter.
    Fields {
        ranges: Vec<(usize, usize)>,
        delim_char: char,
    },
}

/// Parse a range list string such as `1,3-5,7-`.
///
/// Each range is a pair `(start, end)` where both bounds are 1‑based and
/// inclusive.  An open‑ended range like `7-` uses `usize::MAX` as the end.
/// The list can be separated by commas or blanks (whitespace).
/// Returns an error if the list contains invalid elements (e.g. zero,
/// non-numeric characters, or decreasing ranges).
fn parse_list(s: &str) -> Result<Vec<(usize, usize)>, String> {
    let mut result = Vec::new();
    for part in s.split(|c: char| c == ',' || c.is_whitespace()) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((a, b)) = part.split_once('-') {
            let start = if a.is_empty() {
                1
            } else {
                let n = a
                    .parse::<usize>()
                    .map_err(|_| "invalid byte, character or field list".to_string())?;
                if n == 0 {
                    return Err("invalid byte, character or field list".to_string());
                }
                n
            };
            let end = if b.is_empty() {
                usize::MAX
            } else {
                let n = b
                    .parse::<usize>()
                    .map_err(|_| "invalid byte, character or field list".to_string())?;
                if n == 0 {
                    return Err("invalid byte, character or field list".to_string());
                }
                n
            };
            if start > end {
                return Err("invalid byte, character or field list".to_string());
            }
            result.push((start, end));
        } else {
            let n = part
                .parse::<usize>()
                .map_err(|_| "invalid byte, character or field list".to_string())?;
            if n == 0 {
                return Err("invalid byte, character or field list".to_string());
            }
            result.push((n, n));
        }
    }
    Ok(result)
}

/// Sort ranges and merge overlapping or directly adjacent ones.
///
/// Open‑ended ranges (end = `usize::MAX`) will stay as the last range if
/// present.  The result is a minimal list of non‑overlapping, strictly
/// increasing intervals.
fn canonical_ranges(ranges: &[(usize, usize)]) -> Vec<(usize, usize)> {
    if ranges.is_empty() {
        return Vec::new();
    }

    let mut sorted = ranges.to_vec();
    sorted.sort_unstable_by_key(|r| r.0);

    let mut merged = Vec::with_capacity(sorted.len());
    let (mut cur_start, mut cur_end) = sorted[0];

    for &(s, e) in &sorted[1..] {
        if s <= cur_end.saturating_add(1) {
            // overlap or adjacent – extend current interval
            cur_end = cur_end.max(e);
        } else {
            merged.push((cur_start, cur_end));
            cur_start = s;
            cur_end = e;
        }
    }
    merged.push((cur_start, cur_end));
    merged
}

/// Process a single input file (or stdin when `file == "-"`) and emit the
/// selected portions of each line.
///
/// The output is written through a buffered, locked stdout handle for maximum
/// throughput.  Only the required parts of each line are traversed.
fn cut_file(file: &str, mode: &Mode) -> Result<(), String> {
    let reader: Box<dyn BufRead> = if file == "-" {
        Box::new(std::io::stdin().lock())
    } else {
        let f = File::open(file).map_err(|e| format!("'{file}': {e}"))?;
        Box::new(BufReader::new(f))
    };

    let stdout = std::io::stdout();
    let mut writer = BufWriter::new(stdout.lock());

    // Reusable output buffer — cleared and reused for each line.
    let mut out = Vec::with_capacity(4096);

    match mode {
        Mode::Bytes(ranges) => {
            for line in reader.lines() {
                let line = line.map_err(|e| e.to_string())?;
                let bytes = line.as_bytes();
                out.clear();
                for &(start, end) in ranges {
                    let s_idx = start.saturating_sub(1);
                    let e_idx = std::cmp::min(end, bytes.len());
                    if s_idx < e_idx {
                        out.extend_from_slice(&bytes[s_idx..e_idx]);
                    }
                }
                writer.write_all(&out).map_err(|e| e.to_string())?;
                writer.write_all(b"\n").map_err(|e| e.to_string())?;
            }
        }
        Mode::Chars(ranges) => {
            for line in reader.lines() {
                let line = line.map_err(|e| e.to_string())?;
                out.clear();
                let mut range_iter = ranges.iter().copied().peekable();
                let mut current: Option<(usize, usize)> = None;

                for (char_idx, (byte_start, c)) in line.char_indices().enumerate() {
                    let idx_1 = char_idx + 1; // 1‑based character index

                    // Advance the state machine to the right range for idx_1.
                    loop {
                        match current {
                            Some((_, end)) if idx_1 > end => {
                                current = None;
                                continue;
                            }
                            Some(_) => break,
                            None => match range_iter.peek() {
                                Some(&(s, e)) => {
                                    if idx_1 < s {
                                        break;
                                    }
                                    current = Some((s, e));
                                    range_iter.next();
                                    continue;
                                }
                                None => break,
                            },
                        }
                    }

                    if let Some((start, _)) = current {
                        if idx_1 >= start {
                            // Copy the UTF‑8 bytes of this character.
                            let char_end = byte_start + c.len_utf8();
                            out.extend_from_slice(&line.as_bytes()[byte_start..char_end]);
                        }
                    }

                    // Stop early when there are no more ranges to cover.
                    if current.is_none() && range_iter.peek().is_none() {
                        break;
                    }
                }

                writer.write_all(&out).map_err(|e| e.to_string())?;
                writer.write_all(b"\n").map_err(|e| e.to_string())?;
            }
        }
        Mode::Fields { ranges, delim_char } => {
            for line in reader.lines() {
                let line = line.map_err(|e| e.to_string())?;
                out.clear();

                // POSIX: "Lines with no field delimiters shall be passed through intact"
                if !line.contains(*delim_char) {
                    out.extend_from_slice(line.as_bytes());
                    writer.write_all(&out).map_err(|e| e.to_string())?;
                    writer.write_all(b"\n").map_err(|e| e.to_string())?;
                    continue;
                }

                let mut range_iter = ranges.iter().copied().peekable();
                let mut current: Option<(usize, usize)> = None;
                let mut first_field = true;

                let mut delim_buf = [0u8; 4];
                let delim_bytes = delim_char.encode_utf8(&mut delim_buf).as_bytes();

                for (idx_0, field) in line.split(*delim_char).enumerate() {
                    let idx_1 = idx_0 + 1;

                    loop {
                        match current {
                            Some((_, end)) if idx_1 > end => {
                                current = None;
                                continue;
                            }
                            Some(_) => break,
                            None => match range_iter.peek() {
                                Some(&(s, e)) => {
                                    if idx_1 < s {
                                        break;
                                    }
                                    current = Some((s, e));
                                    range_iter.next();
                                    continue;
                                }
                                None => break,
                            },
                        }
                    }

                    if let Some((start, _)) = current {
                        if idx_1 >= start {
                            if !first_field {
                                out.extend_from_slice(delim_bytes);
                            }
                            out.extend_from_slice(field.as_bytes());
                            first_field = false;
                        }
                    }

                    if current.is_none() && range_iter.peek().is_none() {
                        break;
                    }
                }

                writer.write_all(&out).map_err(|e| e.to_string())?;
                writer.write_all(b"\n").map_err(|e| e.to_string())?;
            }
        }
    }

    writer.flush().map_err(|e| e.to_string())?;
    Ok(())
}

register_command!(
    CUT_CMD,
    "cut",
    "b:c:f:d:",
    CommandFlags::BIN.bits(),
    cut_main,
    description = "Extract selected parts of each line of a file",
    help = "\
OPTIONS:
-b LIST   Select bytes.
-c LIST   Select characters.
-f LIST   Select fields (delimited by -d, default TAB).
-d DELIM  Field delimiter (used with -f)."
);
