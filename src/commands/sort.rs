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
use std::io::Read;

/// A sort key that can be compared cheaply.
/// Built once per line before sorting (Schwartzian transform).
#[derive(Debug, Clone)]
enum SortKey {
    /// Numeric key (from -n).
    Numeric(f64),
    /// Borrowed string slice — used when no case-folding or field extraction
    /// creates new owned strings. Points into the original line buffer.
    Borrowed(StringRef),
    /// Owned string — used when -f or -k requires a modified copy.
    Owned(OwnedStr),
}

/// Thin wrapper: two indices delimiting a slice inside the original line.
#[derive(Debug, Clone, Copy)]
struct StringRef {
    start: usize,
    end: usize,
}

/// Owned string kept on the heap, allocated once per line when necessary.
#[derive(Debug, Clone)]
struct OwnedStr(String);

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
    let keys = opts.get_str('k').unwrap_or("").to_string();

    let args: Vec<String> = ctx.optargs.clone();
    let files: Vec<String> = if args.is_empty() {
        vec!["-".to_string()]
    } else {
        args
    };

    // Collect all input lines efficiently.
    let mut lines: Vec<String> = Vec::new();
    let mut exit_code: u8 = 0;

    for file in &files {
        match read_lines_eager(file) {
            Ok(mut l) => lines.append(&mut l),
            Err(e) => {
                eprintln!("sort: {e}");
                exit_code = 1;
            }
        }
    }

    // Early exit on empty input.
    if lines.is_empty() {
        return exit_code;
    }

    // Parse the sort key (simplified: a single field number).
    let key = parse_key(&keys);

    // Determine if we can use the fast path: no -n, no -f, no -k, no -u.
    let fast_path = !flag_n && !flag_f && key.is_none();

    if fast_path {
        // Fast path: sort string slices directly, no key building needed.
        if flag_r {
            lines.sort_unstable_by(|a, b| b.cmp(a));
        } else {
            lines.sort_unstable();
        }

        if flag_u {
            lines.dedup();
        }
    } else {
        // General path: Schwartzian transform with pre-built keys.
        let mut decorated: Vec<Decorated> = lines
            .into_iter()
            .map(|line| {
                let sort_key = build_sort_key(&line, flag_n, flag_f, &key);
                Decorated { key: sort_key, line }
            })
            .collect();

        // Sort by the key, respecting -r.
        decorated.sort_by(|a, b| compare_keys(&a.key, &b.key, &a.line, &b.line, flag_r));

        // Deduplicate consecutive equal lines when -u is requested.
        if flag_u {
            decorated.dedup_by(|a, b| keys_equal(&a.key, &b.key, &a.line, &b.line));
        }

        // Output.
        for d in &decorated {
            println!("{}", d.line);
        }
        return exit_code;
    }

    // Output for fast path.
    for line in &lines {
        println!("{}", line);
    }

    exit_code
}

/// Compare two sort keys, using the original lines when the key borrows from them.
#[inline]
fn compare_keys(
    a: &SortKey,
    b: &SortKey,
    line_a: &str,
    line_b: &str,
    reverse: bool,
) -> Ordering {
    let ord = match (a, b) {
        (SortKey::Numeric(na), SortKey::Numeric(nb)) => {
            na.partial_cmp(nb).unwrap_or(Ordering::Equal)
        }
        (SortKey::Borrowed(ref_a), SortKey::Borrowed(ref_b)) => {
            let sa = &line_a[ref_a.start..ref_a.end];
            let sb = &line_b[ref_b.start..ref_b.end];
            sa.cmp(sb)
        }
        (SortKey::Owned(OwnedStr(sa)), SortKey::Owned(OwnedStr(sb))) => sa.cmp(sb),
        // Mixed: numbers sort before strings.
        (SortKey::Numeric(_), _) => Ordering::Less,
        (_, SortKey::Numeric(_)) => Ordering::Greater,
        // Mixed Borrowed/Owned — treat both as strings.
        (a, b) => {
            let sa = resolve_str(a, line_a);
            let sb = resolve_str(b, line_b);
            sa.as_ref().cmp(sb.as_ref())
        }
    };

    if reverse {
        ord.reverse()
    } else {
        ord
    }
}

/// Check equality of two sort keys (for dedup).
#[inline]
fn keys_equal(a: &SortKey, b: &SortKey, line_a: &str, line_b: &str) -> bool {
    match (a, b) {
        (SortKey::Numeric(na), SortKey::Numeric(nb)) => na == nb,
        (SortKey::Borrowed(ref_a), SortKey::Borrowed(ref_b)) => {
            &line_a[ref_a.start..ref_a.end] == &line_b[ref_b.start..ref_b.end]
        }
        (SortKey::Owned(OwnedStr(sa)), SortKey::Owned(OwnedStr(sb))) => sa == sb,
        _ => {
            let sa = resolve_str(a, line_a);
            let sb = resolve_str(b, line_b);
            sa.as_ref() == sb.as_ref()
        }
    }
}

/// Get a &str from a SortKey, using the original line for Borrowed variants.
#[inline]
fn resolve_str<'a>(key: &'a SortKey, line: &'a str) -> std::borrow::Cow<'a, str> {
    match key {
        SortKey::Borrowed(r) => std::borrow::Cow::Borrowed(&line[r.start..r.end]),
        SortKey::Owned(OwnedStr(s)) => std::borrow::Cow::Borrowed(s.as_str()),
        SortKey::Numeric(_) => std::borrow::Cow::Borrowed(""),
    }
}

/// Build the sort key for a single line.
fn build_sort_key(
    line: &str,
    numeric: bool,
    fold_case: bool,
    key: &Option<(usize, usize)>,
) -> SortKey {
    // Find the field boundaries in the original string.
    let (field_start, field_end) = match key {
        Some((start, _)) => field_range(line, *start),
        None => (0, line.len()),
    };

    // If we need numeric parsing, always parse from the slice.
    if numeric {
        let slice = &line[field_start..field_end];
        let val = slice.trim().parse::<f64>().unwrap_or(0.0);
        return SortKey::Numeric(val);
    }

    // If case-folding is needed, we must allocate an owned lowercased string.
    if fold_case {
        let lower = line[field_start..field_end].to_lowercase();
        return SortKey::Owned(OwnedStr(lower));
    }

    // Neither -n nor -f: we can borrow from the original line (zero-copy).
    SortKey::Borrowed(StringRef {
        start: field_start,
        end: field_end,
    })
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
            // Entering a new field.
            field_idx += 1;
            if field_idx == n {
                start = i;
            }
            in_field = true;
        } else if is_space && in_field {
            // Leaving a field.
            if field_idx == n {
                return (start, i);
            }
            in_field = false;
        }
    }

    // Last field extends to end of line.
    if field_idx == n && in_field {
        return (start, line.len());
    }

    // Field not found.
    (0, 0)
}

/// Read all lines from a file (or stdin) into a Vec, pre-allocating capacity.
fn read_lines_eager(file: &str) -> Result<Vec<String>, String> {
    let mut content = String::new();

    if file == "-" {
        std::io::stdin()
            .read_to_string(&mut content)
            .map_err(|e| format!("stdin: {}", e))?;
    } else {
        let mut f = File::open(file).map_err(|e| format!("'{}': {}", file, e))?;
        f.read_to_string(&mut content)
            .map_err(|e| format!("'{}': {}", file, e))?;
    }

    // Count lines for pre-allocation.
    let line_count = content.bytes().filter(|&b| b == b'\n').count();
    let mut lines: Vec<String> = Vec::with_capacity(line_count);

    let mut start = 0;
    for (i, ch) in content.char_indices() {
        if ch == '\n' {
            // Strip trailing \r for Windows line endings.
            let end = if i > start && content.as_bytes()[i - 1] == b'\r' {
                i - 1
            } else {
                i
            };
            lines.push(content[start..end].to_string());
            start = i + 1;
        }
    }

    // Last line without trailing newline.
    if start < content.len() {
        lines.push(content[start..].to_string());
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

    let parts: Vec<&str> = s.split(',').collect();
    let start = parts[0].parse::<usize>().ok()?;
    let end = if parts.len() > 1 {
        parts[1].parse::<usize>().unwrap_or(usize::MAX)
    } else {
        usize::MAX
    };

    Some((start, end))
}

register_command!(
    SORT_CMD,
    "sort",
    "rnufk:",
    CommandFlags::BIN.bits(),
    sort_main
);