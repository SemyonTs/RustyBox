// =============================================================================
// sort — Sort lines of text files.
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
//   -r        Reverse the comparison order.
//   -n        Compare according to numeric value.
//   -u        Unique: output only the first of equal lines.
//   -f        Case-insensitive comparison.
//   -k KEYDEF Sort by a specific field (simplified: single field number).
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::cmp::Ordering;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};

/// A sort key that can be compared cheaply.
/// Built once per line before sorting (Schwartzian transform).
enum SortKey {
    /// Numeric key (from -n).
    Numeric(f64),
    /// Borrowed string slice — points directly into the original line buffer.
    Borrowed(usize, usize),
    /// Owned string — used when -f requires a lowercased copy.
    Owned(String),
}

/// Holds both the pre-built sort key and the original line.
struct Decorated {
    key: SortKey,
    line: String,
}

/// Entry point for the `sort` builtin.
///
/// When no file arguments are given stdin is read.
fn sort_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "rnufk:") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("sort: {e}");
            return 1;
        }
    };

    let flag_r = opts.count('r') > 0;
    let flag_n = opts.count('n') > 0;
    let flag_u = opts.count('u') > 0;
    let flag_f = opts.count('f') > 0;
    let keys = opts.get_str('k').unwrap_or("");

    // Collect all input lines without cloning the entire optargs vector.
    let mut lines: Vec<String> = Vec::new();
    let mut exit_code: u8 = 0;

    if ctx.optargs.is_empty() {
        match read_lines("-") {
            Ok(mut l) => lines.append(&mut l),
            Err(e) => {
                eprintln!("sort: {e}");
                exit_code = 1;
            }
        }
    } else {
        for file in &ctx.optargs {
            match read_lines(file) {
                Ok(mut l) => lines.append(&mut l),
                Err(e) => {
                    eprintln!("sort: {e}");
                    exit_code = 1;
                }
            }
        }
    }

    // Early exit on empty input.
    if lines.is_empty() {
        return exit_code;
    }

    // Parse the sort key (simplified: a single field number).
    let key = parse_key(keys);

    // Determine if we can use the fast path: no -n, no -f, no -k.
    let fast_path = !flag_n && !flag_f && key.is_none();

    let stdout = std::io::stdout();
    let mut writer = BufWriter::new(stdout.lock());

    if fast_path {
        // Fast path: sort string slices directly.
        if flag_r {
            lines.sort_unstable_by(|a, b| b.cmp(a));
        } else {
            lines.sort_unstable();
        }

        if flag_u {
            lines.dedup();
        }

        for line in &lines {
            writeln!(writer, "{}", line).ok();
        }
    } else {
        // General path: Schwartzian transform with pre-built keys.
        let mut decorated: Vec<Decorated> = lines
            .into_iter()
            .map(|line| {
                let sort_key = build_sort_key(&line, flag_n, flag_f, &key);
                Decorated {
                    key: sort_key,
                    line,
                }
            })
            .collect();

        // Sort by the key, respecting -r.
        decorated.sort_unstable_by(|a, b| {
            let ord = compare_keys(&a.key, &b.key, &a.line, &b.line);
            if flag_r { ord.reverse() } else { ord }
        });

        // Deduplicate consecutive equal lines when -u is requested.
        if flag_u {
            decorated.dedup_by(|a, b| keys_equal(&a.key, &b.key, &a.line, &b.line));
        }

        for d in &decorated {
            writeln!(writer, "{}", d.line).ok();
        }
    }

    writer.flush().ok();
    exit_code
}

/// Compare two sort keys, using the original lines when the key borrows from them.
fn compare_keys(a: &SortKey, b: &SortKey, line_a: &str, line_b: &str) -> Ordering {
    match (a, b) {
        (SortKey::Numeric(na), SortKey::Numeric(nb)) => {
            na.partial_cmp(nb).unwrap_or(Ordering::Equal)
        }
        (SortKey::Borrowed(sa, ea), SortKey::Borrowed(sb, eb)) => {
            line_a[*sa..*ea].cmp(&line_b[*sb..*eb])
        }
        (SortKey::Owned(sa), SortKey::Owned(sb)) => sa.cmp(sb),
        (SortKey::Numeric(_), _) => Ordering::Less,
        (_, SortKey::Numeric(_)) => Ordering::Greater,
        (a, b) => resolve_str(a, line_a).cmp(resolve_str(b, line_b)),
    }
}

/// Check equality of two sort keys (for dedup).
fn keys_equal(a: &SortKey, b: &SortKey, line_a: &str, line_b: &str) -> bool {
    match (a, b) {
        (SortKey::Numeric(na), SortKey::Numeric(nb)) => na == nb,
        (SortKey::Borrowed(sa, ea), SortKey::Borrowed(sb, eb)) => {
            line_a[*sa..*ea] == line_b[*sb..*eb]
        }
        (SortKey::Owned(sa), SortKey::Owned(sb)) => sa == sb,
        (a, b) => resolve_str(a, line_a) == resolve_str(b, line_b),
    }
}

/// Get a &str from a SortKey, using the original line for Borrowed variants.
fn resolve_str<'a>(key: &'a SortKey, line: &'a str) -> &'a str {
    match key {
        SortKey::Borrowed(s, e) => &line[*s..*e],
        SortKey::Owned(s) => s.as_str(),
        SortKey::Numeric(_) => "",
    }
}

/// Build the sort key for a single line.
fn build_sort_key(
    line: &str,
    numeric: bool,
    fold_case: bool,
    key: &Option<(usize, usize)>,
) -> SortKey {
    let (field_start, field_end) = match key {
        Some((start, _)) => field_range(line, *start),
        None => (0, line.len()),
    };

    if numeric {
        let slice = &line[field_start..field_end];
        let val = slice.trim().parse::<f64>().unwrap_or(0.0);
        return SortKey::Numeric(val);
    }

    if fold_case {
        let lower = line[field_start..field_end].to_lowercase();
        return SortKey::Owned(lower);
    }

    SortKey::Borrowed(field_start, field_end)
}

/// Return the byte range of the n-th whitespace-delimited field (1-based).
/// If n == 0, returns the whole line. If the field doesn't exist, returns (0, 0).
fn field_range(line: &str, n: usize) -> (usize, usize) {
    if n == 0 {
        return (0, line.len());
    }

    let mut field_idx = 0;
    let mut start = 0;
    let mut in_field = false;

    for (i, ch) in line.char_indices() {
        let is_space = ch.is_whitespace();
        if !is_space && !in_field {
            field_idx += 1;
            if field_idx == n {
                start = i;
            }
            in_field = true;
        } else if is_space && in_field {
            if field_idx == n {
                return (start, i);
            }
            in_field = false;
        }
    }

    if field_idx == n && in_field {
        return (start, line.len());
    }

    (0, 0)
}

/// Read all lines from a file (or stdin) into a Vec using `read_line` for
/// reuse of a single buffer — avoids allocating an intermediate String for
/// the whole file content.
fn read_lines(file: &str) -> Result<Vec<String>, String> {
    let mut reader: Box<dyn BufRead> = if file == "-" {
        Box::new(std::io::stdin().lock())
    } else {
        let f = File::open(file).map_err(|e| format!("'{}': {}", file, e))?;
        Box::new(BufReader::new(f))
    };

    let mut lines = Vec::new();
    let mut buf = String::with_capacity(4096);

    loop {
        buf.clear();
        let n = reader
            .read_line(&mut buf)
            .map_err(|e| format!("'{}': {}", file, e))?;
        if n == 0 {
            break;
        }
        // Strip trailing newline (and optional \r for CRLF).
        let line = if buf.ends_with('\n') {
            let end = buf.len() - 1;
            if end > 0 && buf.as_bytes()[end - 1] == b'\r' {
                buf[..end - 1].to_string()
            } else {
                buf[..end].to_string()
            }
        } else {
            buf.clone()
        };
        lines.push(line);
    }

    Ok(lines)
}

/// Parse a key definition string of the form `start[,end]`.
///
/// Field numbers are 1-based.  Returns `None` when the string is empty.
fn parse_key(s: &str) -> Option<(usize, usize)> {
    if s.is_empty() {
        return None;
    }

    let mut parts = s.split(',');
    let start = parts.next()?.parse::<usize>().ok()?;
    let end = parts
        .next()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(usize::MAX);

    Some((start, end))
}

register_command!(
    SORT_CMD,
    "sort",
    "rnufk:",
    CommandFlags::BIN.bits(),
    sort_main
);
