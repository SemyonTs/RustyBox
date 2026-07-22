// =============================================================================
// wc — Print newline, word, and byte counts for each file.
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
//   -l   Print the newline count.
//   -w   Print the word count.
//   -c   Print the byte count.
//   -m   Print the character count (may differ from -c for multibyte text).
//
// When no options are given the default is `-lwc`.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::fs::File;
use std::io::{BufRead, BufReader};

/// Entry point for the `wc` builtin.
///
/// Reads from stdin when no file arguments are supplied.
fn wc_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "lwc(m)") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("wc: {e}");
            return 1;
        }
    };

    let mut flag_l = opts.count('l') > 0;
    let mut flag_w = opts.count('w') > 0;
    let mut flag_c = opts.count('c') > 0;
    let flag_m = opts.count('m') > 0;

    // Default: show lines, words, and bytes when no flag is set.
    if !flag_l && !flag_w && !flag_c && !flag_m {
        flag_l = true;
        flag_w = true;
        flag_c = true;
    }

    let args: Vec<String> = ctx.optargs.clone();
    let files: Vec<String> = if args.is_empty() {
        vec!["-".to_string()]
    } else {
        args
    };

    let mut total_lines = 0u64;
    let mut total_words = 0u64;
    let mut total_bytes = 0u64;
    let mut total_chars = 0u64;
    let mut exit_code: u8 = 0;
    let multiple = files.len() > 1;

    for file in &files {
        match wc_file(file, flag_l, flag_w, flag_c, flag_m) {
            Ok((l, w, c, ch)) => {
                print_counts(
                    file, l, w, c, ch, flag_l, flag_w, flag_c, flag_m,
                    multiple,
                );
                total_lines += l;
                total_words += w;
                total_bytes += c;
                total_chars += ch;
            }
            Err(e) => {
                eprintln!("wc: {e}");
                exit_code = 1;
            }
        }
    }

    // Emit a cumulative total line when more than one file was processed.
    if multiple {
        print_counts(
            "total",
            total_lines,
            total_words,
            total_bytes,
            total_chars,
            flag_l,
            flag_w,
            flag_c,
            flag_m,
            false,
        );
    }

    exit_code
}

/// Count lines, words, bytes, and optionally characters in a single file
/// (or stdin when `file == "-"`).
fn wc_file(
    file: &str,
    _flag_l: bool,
    _flag_w: bool,
    _flag_c: bool,
    flag_m: bool,
) -> Result<(u64, u64, u64, u64), String> {
    let mut lines = 0u64;
    let mut words = 0u64;
    let mut bytes = 0u64;
    let mut chars = 0u64;

    if file == "-" {
        let stdin = std::io::stdin();
        let reader = stdin.lock();
        for line in reader.lines() {
            let line = line.map_err(|e| e.to_string())?;
            lines += 1;
            words += count_words(&line);
            bytes += line.len() as u64 + 1; // +1 for the newline character.
            if flag_m {
                chars += line.chars().count() as u64 + 1;
            }
        }
    } else {
        let f = File::open(file).map_err(|e| format!("'{}': {}", file, e))?;
        let mut reader = BufReader::new(f);
        let mut buf = String::new();

        loop {
            buf.clear();
            let n = reader
                .read_line(&mut buf)
                .map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            lines += 1;
            words += count_words(&buf);
            bytes += n as u64;
            if flag_m {
                chars += buf.chars().count() as u64;
            }
        }
    }

    Ok((lines, words, bytes, chars))
}

/// Count the number of whitespace-delimited words in `s`.
fn count_words(s: &str) -> u64 {
    let mut count = 0u64;
    let mut in_word = false;

    for c in s.chars() {
        if c.is_whitespace() {
            in_word = false;
        } else if !in_word {
            in_word = true;
            count += 1;
        }
    }

    count
}

/// Print one row of counts.
fn print_counts(
    file: &str,
    l: u64,
    w: u64,
    c: u64,
    ch: u64,
    flag_l: bool,
    flag_w: bool,
    flag_c: bool,
    flag_m: bool,
    show_file: bool,
) {
    let mut parts = Vec::new();

    if flag_l {
        parts.push(format!("{:>7}", l));
    }
    if flag_w {
        parts.push(format!("{:>7}", w));
    }
    if flag_m {
        parts.push(format!("{:>7}", ch));
    }
    if flag_c {
        parts.push(format!("{:>7}", c));
    }

    if show_file {
        println!("{} {}", parts.join(" "), file);
    } else {
        println!("{}", parts.join(" "));
    }
}

register_command!(
    WC_CMD,
    "wc",
    "lwc(m)",
    CommandFlags::BIN.bits(),
    wc_main
);