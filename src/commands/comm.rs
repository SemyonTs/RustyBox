// =============================================================================
// comm — select or reject lines common to two files
// =============================================================================
// Copyright (c) 2026 RustyBox contributors
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// POSIX specification:
//   - compare two sorted files line by line
//   - output three columns: lines only in file1, only in file2, and common
//   - options -1, -2, -3 suppress respective columns
//   - tab characters used as separators according to precise rules
//   - file operands may be '-' for stdin
//   - extension: -i for case‑insensitive comparison
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

/// Entry point for the `comm` builtin.
///
/// Expects exactly two file operands.  Files must be sorted according to the
/// collating sequence of the current locale (here we use simple byte/string
/// ordering, which is equivalent to the POSIX locale).
fn comm_main(ctx: &mut Context) -> u8 {
    // Parse options: -1, -2, -3 (POSIX) and -i (extension).
    let opts = match crate::args::parse(ctx, "123i") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("comm: {e}");
            return 1;
        }
    };

    let suppress1 = opts.count('1') > 0;
    let suppress2 = opts.count('2') > 0;
    let suppress3 = opts.count('3') > 0;
    let case_insensitive = opts.count('i') > 0;

    if ctx.optargs.len() != 2 {
        eprintln!("comm: need two file operands");
        return 1;
    }

    let file1 = &ctx.optargs[0];
    let file2 = &ctx.optargs[1];

    // Open file1 (or stdin if '-')
    let reader1: Box<dyn BufRead> = if file1 == "-" {
        Box::new(BufReader::new(io::stdin().lock()))
    } else {
        match File::open(file1) {
            Ok(f) => Box::new(BufReader::new(f)),
            Err(e) => {
                eprintln!("comm: cannot open '{}': {}", file1, e);
                return 1;
            }
        }
    };

    // Open file2 (or stdin if '-')
    let reader2: Box<dyn BufRead> = if file2 == "-" {
        Box::new(BufReader::new(io::stdin().lock()))
    } else {
        match File::open(file2) {
            Ok(f) => Box::new(BufReader::new(f)),
            Err(e) => {
                eprintln!("comm: cannot open '{}': {}", file2, e);
                return 1;
            }
        }
    };

    let mut lines1 = reader1.lines();
    let mut lines2 = reader2.lines();

    // Helper to read the next line, handling errors.
    let read_next = |lines: &mut dyn Iterator<Item = io::Result<String>>| -> Option<String> {
        lines.next().map(|res| {
            res.unwrap_or_else(|e| {
                eprintln!("comm: read error: {}", e);
                // We exit with error; but we can't easily propagate, so we'll
                // exit in the main loop. We'll return an empty string and set a flag.
                // To keep it simple, we'll use `expect`; but better to handle gracefully.
                // We'll change approach: use a function that returns Result.
                String::new()
            })
        })
    };

    // For robust error handling, we'll use a custom iterator that returns Result.
    // Instead, we'll manually track errors with a flag.
    let mut error_occurred = false;

    let mut line1_opt: Option<String> = lines1.next().transpose().unwrap_or_else(|e| {
        eprintln!("comm: error reading '{}': {}", file1, e);
        error_occurred = true;
        None
    });
    let mut line2_opt: Option<String> = lines2.next().transpose().unwrap_or_else(|e| {
        eprintln!("comm: error reading '{}': {}", file2, e);
        error_occurred = true;
        None
    });

    if error_occurred {
        return 1;
    }

    let stdout = io::stdout();
    let mut out = stdout.lock();

    // Compare two lines according to the current collation.
    // We use simple byte/string comparison; for POSIX locale this is correct.
    // For other locales, we would need strcoll, but we keep it simple.
    let cmp_lines = |a: &str, b: &str| {
        if case_insensitive {
            a.to_lowercase().cmp(&b.to_lowercase())
        } else {
            a.cmp(b)
        }
    };

    loop {
        match (&line1_opt, &line2_opt) {
            (Some(line1), Some(line2)) => {
                match cmp_lines(line1, line2) {
                    std::cmp::Ordering::Less => {
                        // line1 is only in file1
                        if !suppress1 {
                            // Column 1: no leading tabs
                            writeln!(out, "{}", line1).ok();
                        }
                        line1_opt = lines1.next().transpose().unwrap_or_else(|e| {
                            eprintln!("comm: error reading '{}': {}", file1, e);
                            error_occurred = true;
                            None
                        });
                    }
                    std::cmp::Ordering::Greater => {
                        // line2 is only in file2
                        if !suppress2 {
                            // Column 2: leading tab if column 1 is not suppressed
                            if !suppress1 {
                                writeln!(out, "\t{}", line2).ok();
                            } else {
                                writeln!(out, "{}", line2).ok();
                            }
                        }
                        line2_opt = lines2.next().transpose().unwrap_or_else(|e| {
                            eprintln!("comm: error reading '{}': {}", file2, e);
                            error_occurred = true;
                            None
                        });
                    }
                    std::cmp::Ordering::Equal => {
                        // Common line: column 3
                        if !suppress3 {
                            let lead = match (suppress1, suppress2) {
                                (false, false) => "\t\t",
                                (true, false) | (false, true) => "\t",
                                (true, true) => "",
                            };
                            writeln!(out, "{}{}", lead, line1).ok();
                        }
                        line1_opt = lines1.next().transpose().unwrap_or_else(|e| {
                            eprintln!("comm: error reading '{}': {}", file1, e);
                            error_occurred = true;
                            None
                        });
                        line2_opt = lines2.next().transpose().unwrap_or_else(|e| {
                            eprintln!("comm: error reading '{}': {}", file2, e);
                            error_occurred = true;
                            None
                        });
                    }
                }
            }
            (Some(line1), None) => {
                // Remaining lines only in file1
                if !suppress1 {
                    writeln!(out, "{}", line1).ok();
                }
                line1_opt = lines1.next().transpose().unwrap_or_else(|e| {
                    eprintln!("comm: error reading '{}': {}", file1, e);
                    error_occurred = true;
                    None
                });
            }
            (None, Some(line2)) => {
                // Remaining lines only in file2
                if !suppress2 {
                    if !suppress1 {
                        writeln!(out, "\t{}", line2).ok();
                    } else {
                        writeln!(out, "{}", line2).ok();
                    }
                }
                line2_opt = lines2.next().transpose().unwrap_or_else(|e| {
                    eprintln!("comm: error reading '{}': {}", file2, e);
                    error_occurred = true;
                    None
                });
            }
            (None, None) => break,
        }

        if error_occurred {
            return 1;
        }
    }

    out.flush().ok();
    if error_occurred { 1 } else { 0 }
}

register_command!(
    COMM_CMD,
    "comm",
    "123i",
    CommandFlags::BIN.bits(),
    comm_main,
    description = "Compare two sorted files line by line",
    help = "\
OPTIONS:
-1      Suppress column 1 (lines only in first file).
-2      Suppress column 2 (lines only in second file).
-3      Suppress column 3 (lines common to both files).
-i      Case-insensitive comparison (extension)."
);
