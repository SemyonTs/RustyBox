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

    let bytes = opts.get_str('b').unwrap_or("").to_string();
    let chars = opts.get_str('c').unwrap_or("").to_string();
    let fields = opts.get_str('f').unwrap_or("").to_string();
    let delim_str = opts.get_str('d').unwrap_or("\t").to_string();

    let mode = if !bytes.is_empty() {
        Mode::Bytes(canonical_ranges(&parse_list(&bytes)))
    } else if !chars.is_empty() {
        Mode::Chars(canonical_ranges(&parse_list(&chars)))
    } else if !fields.is_empty() {
        let delim_char = delim_str.chars().next().unwrap_or('\t');
        Mode::Fields {
            ranges: canonical_ranges(&parse_list(&fields)),
            delim_char,
            delim_str,
        }
    } else {
        eprintln!("cut: need to specify -b, -c or -f");
        return 1;
    };

    let args: Vec<String> = ctx.optargs.clone();
    let files: Vec<String> = if args.is_empty() {
        vec!["-".to_string()]
    } else {
        args
    };

    let mut exit_code: u8 = 0;
    for file in &files {
        if let Err(e) = cut_file(file, &mode) {
            eprintln!("cut: {e}");
            exit_code = 1;
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
        delim_str: String,
    },
}

/// Parse a range list string such as `1,3-5,7-`.
///
/// Each range is a pair `(start, end)` where both bounds are 1‑based and
/// inclusive.  An open‑ended range like `7-` uses `usize::MAX` as the end.
fn parse_list(s: &str) -> Vec<(usize, usize)> {
    let mut result = Vec::new();
    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((a, b)) = part.split_once('-') {
            let start = a.parse::<usize>().unwrap_or(1);
            let end = if b.is_empty() {
                usize::MAX
            } else {
                b.parse::<usize>().unwrap_or(usize::MAX)
            };
            result.push((start, end));
        } else if let Ok(n) = part.parse::<usize>() {
            result.push((n, n));
        }
    }
    result
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

    match mode {
        Mode::Bytes(ranges) => {
            for line in reader.lines() {
                let line = line.map_err(|e| e.to_string())?;
                let bytes = line.as_bytes();
                let mut out = Vec::with_capacity(bytes.len() / 8); // reasonable guess
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
                let mut out = Vec::with_capacity(line.len() / 4);
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
        Mode::Fields {
            ranges,
            delim_char,
            delim_str,
        } => {
            for line in reader.lines() {
                let line = line.map_err(|e| e.to_string())?;
                let mut out_fields = Vec::with_capacity(ranges.len());
                let mut range_iter = ranges.iter().copied().peekable();
                let mut current: Option<(usize, usize)> = None;

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
                            out_fields.push(field);
                        }
                    }

                    if current.is_none() && range_iter.peek().is_none() {
                        break;
                    }
                }

                // Manually join the selected fields using the original delimiter.
                let mut out = Vec::new();
                for (i, f) in out_fields.iter().enumerate() {
                    if i > 0 {
                        out.extend_from_slice(delim_str.as_bytes());
                    }
                    out.extend_from_slice(f.as_bytes());
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
    cut_main
);