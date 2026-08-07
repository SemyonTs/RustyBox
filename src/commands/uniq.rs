// =============================================================================
// uniq — Report or omit repeated lines.
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
//   -c        Prefix lines by the number of occurrences.
//   -d        Only print duplicate lines (one per group).
//   -u        Only print unique lines.
//   -i        Ignore case when comparing.
//   -f N      Skip first N fields (fields are separated by whitespace).
//   -s N      Skip first N characters.
//   -w N      Compare only N characters (default: compare full line).
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};

/// Entry point for the `uniq` builtin.
fn uniq_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "cduif:s:w:[-cdu]") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("uniq: {e}");
            return 1;
        }
    };

    let flag_c = opts.count('c') > 0;
    let flag_d = opts.count('d') > 0;
    let flag_u = opts.count('u') > 0;
    let flag_i = opts.count('i') > 0;
    let skip_fields_count = opts.get_int('f').unwrap_or(0) as usize;
    let skip_chars_count = opts.get_int('s').unwrap_or(0) as usize;
    let compare_chars_count = opts.get_int('w').unwrap_or(0) as usize;

    // Determine input and output files.
    let mut args = ctx.optargs.iter();
    let input_file = args.next().map(|s| s.as_str());
    let output_file = args.next().map(|s| s.as_str());

    // Open input.
    let input: Box<dyn BufRead> = match input_file {
        Some("-") | None => Box::new(BufReader::new(io::stdin())),
        Some(name) => {
            let file = match File::open(name) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("uniq: {}: {}", name, e);
                    return 1;
                }
            };
            Box::new(BufReader::new(file))
        }
    };

    // Open output.
    let mut output: Box<dyn Write> = match output_file {
        Some("-") | None => Box::new(io::stdout()),
        Some(name) => {
            let file = match File::create(name) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("uniq: {}: {}", name, e);
                    return 1;
                }
            };
            Box::new(file)
        }
    };

    // Process lines.
    let mut lines = input.lines();
    let mut current_line: Option<String> = None;
    let mut count = 0u64;

    // Helper to compare two strings according to options.
    let equal = |a: &str, b: &str| -> bool {
        let a = if flag_i {
            a.to_lowercase()
        } else {
            a.to_string()
        };
        let b = if flag_i {
            b.to_lowercase()
        } else {
            b.to_string()
        };
        let a_after_fields = skip_fields(&a, skip_fields_count);
        let b_after_fields = skip_fields(&b, skip_fields_count);
        let a_after_chars = skip_chars(a_after_fields, skip_chars_count);
        let b_after_chars = skip_chars(b_after_fields, skip_chars_count);
        let a_compare = truncate(a_after_chars, compare_chars_count);
        let b_compare = truncate(b_after_chars, compare_chars_count);
        a_compare == b_compare
    };

    for line_res in lines {
        let line = match line_res {
            Ok(l) => l,
            Err(e) => {
                eprintln!("uniq: read error: {}", e);
                return 1;
            }
        };

        if let Some(ref cur) = current_line {
            if equal(cur, &line) {
                count += 1;
            } else {
                // Output previous group.
                if let Err(e) = output_group(&mut output, cur, count, flag_c, flag_d, flag_u) {
                    eprintln!("uniq: write error: {}", e);
                    return 1;
                }
                // Start new group.
                current_line = Some(line);
                count = 1;
            }
        } else {
            current_line = Some(line);
            count = 1;
        }
    }

    // Output last group.
    if let Some(cur) = current_line {
        if let Err(e) = output_group(&mut output, &cur, count, flag_c, flag_d, flag_u) {
            eprintln!("uniq: write error: {}", e);
            return 1;
        }
    }

    if let Err(e) = output.flush() {
        eprintln!("uniq: flush error: {}", e);
        return 1;
    }

    0
}

/// Write a group of identical lines according to the active flags.
fn output_group<W: Write>(
    output: &mut W,
    line: &str,
    count: u64,
    flag_c: bool,
    flag_d: bool,
    flag_u: bool,
) -> io::Result<()> {
    if flag_c {
        writeln!(output, "{:>7} {}", count, line)?;
    } else if flag_d && count > 1 {
        writeln!(output, "{}", line)?;
    } else if flag_u && count == 1 {
        writeln!(output, "{}", line)?;
    } else if !flag_d && !flag_u {
        // Default: print each line once.
        writeln!(output, "{}", line)?;
    }
    Ok(())
}

/// Skip the first `n` fields (whitespace‑separated) from a string.
fn skip_fields(s: &str, n: usize) -> &str {
    if n == 0 {
        return s;
    }
    let mut fields = 0;
    let mut chars = s.chars();
    let mut pos = 0;
    while fields < n {
        // Skip whitespace.
        while let Some(c) = chars.next() {
            pos += c.len_utf8();
            if !c.is_whitespace() {
                break;
            }
        }
        // Skip non-whitespace (the field).
        while let Some(c) = chars.next() {
            pos += c.len_utf8();
            if c.is_whitespace() {
                break;
            }
        }
        fields += 1;
    }
    &s[pos..]
}

/// Skip the first `n` characters from a string.
fn skip_chars(s: &str, n: usize) -> &str {
    if n == 0 {
        return s;
    }
    let mut count = 0;
    let mut byte_idx = 0;
    for (idx, _) in s.char_indices() {
        if count >= n {
            byte_idx = idx;
            break;
        }
        count += 1;
    }
    if count < n {
        byte_idx = s.len();
    }
    &s[byte_idx..]
}

/// Truncate a string to at most `n` characters.
fn truncate(s: &str, n: usize) -> &str {
    if n == 0 {
        return s;
    }
    let mut count = 0;
    let mut byte_idx = 0;
    for (idx, _) in s.char_indices() {
        if count >= n {
            byte_idx = idx;
            break;
        }
        count += 1;
    }
    if count < n {
        byte_idx = s.len();
    }
    &s[..byte_idx]
}

register_command!(
    UNIQ_CMD,
    "uniq",
    "cduif:s:w:[-cdu]",
    CommandFlags::BIN.bits(),
    uniq_main,
    description = "Report or omit repeated lines",
    help = "\
OPTIONS:
-c  Prefix lines by the number of occurrences.
-d  Only print duplicate lines (one per group).
-u  Only print unique lines.
-i  Ignore case when comparing.
-f N  Skip first N fields (fields are separated by whitespace).
-s N  Skip first N characters.
-w N  Compare only N characters (default: compare full line).
"
);
