// =============================================================================
// expand — Convert tabs to spaces.
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Implementation inspired by Toybox (https://landley.net/toybox/)
// Toybox is copyrighted by Rob Landley, see NOTICE file for license details.
//
// Supported options (POSIX plus -i extension):
//   -t tablist     Specify tab stops.  tablist is either a single positive
//                  integer (tab stops every N columns, default 8) or a list
//                  of two or more positive integers, separated by blanks or
//                  commas, in strictly ascending order.
//   -i             Do not convert tabs after non‑whitespace (i.e. ignore
//                  leading tabs on each line).
//   -              Read from standard input.
//
// This implementation also honours backspace characters: they decrement the
// column position (but not below zero) and are copied verbatim.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};

/// Entry point for the `expand` builtin.
fn expand_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "t:i") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("expand: {e}");
            return 1;
        }
    };

    let flag_i = opts.count('i') > 0;

    // Parse -t argument.
    let (tab_stops, step) = if let Some(t_arg) = opts.get_str('t') {
        parse_tab_stops(t_arg)
    } else {
        (Vec::new(), Some(8))
    };

    // If parsing failed, report error.
    if tab_stops.is_empty() && step.is_none() {
        eprintln!("expand: invalid tab stop specification");
        return 1;
    }

    let sources: Vec<&str> = if ctx.optargs.is_empty() {
        vec!["-"]
    } else {
        ctx.optargs.iter().map(|s| s.as_str()).collect()
    };

    let mut exit_code: u8 = 0;
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for &name in &sources {
        let reader: Box<dyn BufRead> = if name == "-" {
            Box::new(BufReader::new(io::stdin().lock()))
        } else {
            match File::open(name) {
                Ok(f) => Box::new(BufReader::new(f)),
                Err(e) => {
                    eprintln!("expand: {}: {}", name, e);
                    exit_code = 1;
                    continue;
                }
            }
        };

        if let Err(e) = process_reader(reader, &mut out, flag_i, &tab_stops, step) {
            eprintln!("expand: error: {}", e);
            exit_code = 1;
        }
    }

    exit_code
}

/// Parse the -t argument into either a repeating step (if single number)
/// or a list of explicit tab stops.
/// Returns (Vec<usize>, Option<usize>).  If the result is (vec, Some(step)),
/// the step is used for repeating stops.  If it is (vec, None), the vec
/// contains explicit stops.  If both are empty/None, parsing failed.
fn parse_tab_stops(arg: &str) -> (Vec<usize>, Option<usize>) {
    // Check if the argument contains a comma or whitespace (list).
    let has_sep = arg.contains(',') || arg.chars().any(|c| c.is_whitespace());

    if !has_sep {
        // Try as a single positive integer.
        if let Ok(num) = arg.parse::<usize>() {
            if num > 0 {
                return (Vec::new(), Some(num));
            }
        }
        // Invalid single number.
        return (Vec::new(), None);
    }

    // Split by commas and whitespace, ignoring empty parts.
    let mut stops = Vec::new();
    for part in arg.split(|c: char| c == ',' || c.is_whitespace()) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Ok(num) = part.parse::<usize>() {
            if num == 0 {
                return (Vec::new(), None);
            }
            stops.push(num);
        } else {
            return (Vec::new(), None);
        }
    }

    // If after splitting we have only one stop, treat it as a repeating step
    // (though POSIX requires two or more for a list, we handle this gracefully).
    if stops.len() == 1 {
        return (Vec::new(), Some(stops[0]));
    }

    // Ensure strictly ascending order.
    for i in 1..stops.len() {
        if stops[i] <= stops[i - 1] {
            return (Vec::new(), None);
        }
    }

    (stops, None)
}

/// Process a single input source, expanding tabs according to the given stops.
fn process_reader<R: BufRead, W: Write>(
    mut reader: R,
    out: &mut W,
    flag_i: bool,
    tab_stops: &[usize],
    step: Option<usize>,
) -> io::Result<()> {
    let mut line = String::new();
    while reader.read_line(&mut line)? > 0 {
        let mut expanded = String::with_capacity(line.len() * 2);
        let mut col: usize = 0; // current column position (0‑based)
        let mut chars = line.chars().enumerate().peekable();

        // If -i is given, find the first non‑space, non‑tab character.
        let start_idx = if flag_i {
            let mut idx = 0;
            for (i, ch) in line.chars().enumerate() {
                if ch != ' ' && ch != '\t' {
                    idx = i;
                    break;
                }
            }
            idx
        } else {
            0
        };

        while let Some((i, ch)) = chars.next() {
            // Handle backspace: decrement column, copy character.
            if ch == '\x08' {
                if i >= start_idx {
                    // Only if we are past the leading whitespace (for -i)
                    col = col.saturating_sub(1);
                    expanded.push(ch);
                } else {
                    // Still in leading whitespace, just copy and update column.
                    col = col.saturating_sub(1);
                    expanded.push(ch);
                }
                continue;
            }

            if i < start_idx {
                // Leading whitespace region: keep tabs as they are, but still
                // advance the column counter as if they were expanded.
                expanded.push(ch);
                if ch == '\t' {
                    col = next_tab_stop(col, tab_stops, step);
                } else {
                    col += 1;
                }
            } else {
                // Normal region (or after leading whitespace).
                if ch == '\t' {
                    let next_col = next_tab_stop(col, tab_stops, step);
                    let spaces = next_col - col;
                    expanded.push_str(&" ".repeat(spaces));
                    col = next_col;
                } else {
                    expanded.push(ch);
                    col += 1;
                }
            }
        }

        out.write_all(expanded.as_bytes())?;
        line.clear();
    }
    Ok(())
}

/// Given the current column position, return the next tab stop column
/// (1‑based in POSIX, but we use 0‑based column positions internally).
fn next_tab_stop(col: usize, tab_stops: &[usize], step: Option<usize>) -> usize {
    if let Some(step) = step {
        // Repeating stops every `step` columns.
        return ((col / step) + 1) * step;
    }

    // Explicit stops: find the first stop greater than current column.
    for &stop in tab_stops {
        if stop > col {
            return stop;
        }
    }

    // No more stops: replace the tab with a single space (POSIX).
    col + 1
}

register_command!(
    EXPAND_CMD,
    "expand",
    "t:i",
    CommandFlags::BIN.bits(),
    expand_main,
    description = "Convert tabs to spaces",
    help = "\
OPTIONS:
-t tablist       Specify tab stops.  tablist is a single positive integer
                 (repeating stops every N columns, default 8) or a list of
                 two or more positive integers, separated by blanks or commas,
                 in strictly ascending order.
-i               Do not convert tabs after non‑whitespace (ignore leading tabs).
-                Read from standard input."
);
