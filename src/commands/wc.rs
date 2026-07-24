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
use std::io::{BufRead, BufReader, BufWriter, Write};

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

    let mut total_lines = 0u64;
    let mut total_words = 0u64;
    let mut total_bytes = 0u64;
    let mut total_chars = 0u64;
    let mut exit_code: u8 = 0;

    let stdout = std::io::stdout();
    let mut writer = BufWriter::new(stdout.lock());
    let mut out_buf = String::with_capacity(64);

    if ctx.optargs.is_empty() {
        match wc_file("-", flag_m) {
            Ok((l, w, c, ch)) => {
                print_counts(
                    "",
                    l,
                    w,
                    c,
                    ch,
                    flag_l,
                    flag_w,
                    flag_c,
                    flag_m,
                    false,
                    &mut writer,
                    &mut out_buf,
                );
            }
            Err(e) => {
                eprintln!("wc: {e}");
                exit_code = 1;
            }
        }
    } else {
        let multiple = ctx.optargs.len() > 1;

        for file in &ctx.optargs {
            match wc_file(file, flag_m) {
                Ok((l, w, c, ch)) => {
                    print_counts(
                        file,
                        l,
                        w,
                        c,
                        ch,
                        flag_l,
                        flag_w,
                        flag_c,
                        flag_m,
                        multiple,
                        &mut writer,
                        &mut out_buf,
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
                &mut writer,
                &mut out_buf,
            );
        }
    }

    writer.flush().ok();
    exit_code
}

/// Count lines, words, bytes, and optionally characters in a single file
/// (or stdin when `file == "-"`).
fn wc_file(file: &str, flag_m: bool) -> Result<(u64, u64, u64, u64), String> {
    let mut lines = 0u64;
    let mut words = 0u64;
    let mut bytes = 0u64;
    let mut chars = 0u64;

    let mut reader: Box<dyn BufRead> = if file == "-" {
        Box::new(std::io::stdin().lock())
    } else {
        let f = File::open(file).map_err(|e| format!("'{}': {}", file, e))?;
        Box::new(BufReader::new(f))
    };

    // Reusable line buffer — avoids allocating a new String for every line.
    let mut line_buf = String::with_capacity(4096);

    loop {
        line_buf.clear();
        let n = reader.read_line(&mut line_buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        lines += 1;
        words += count_words(&line_buf);
        bytes += n as u64;
        if flag_m {
            chars += line_buf.chars().count() as u64;
        }
    }

    Ok((lines, words, bytes, chars))
}

/// Count the number of whitespace-delimited words in `s`.
fn count_words(s: &str) -> u64 {
    let mut count = 0u64;
    let mut in_word = false;

    for &b in s.as_bytes() {
        if b.is_ascii_whitespace() {
            in_word = false;
        } else if !in_word {
            in_word = true;
            count += 1;
        }
    }

    count
}

/// Print one row of counts directly into a reusable buffer.
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
    writer: &mut BufWriter<std::io::StdoutLock>,
    out_buf: &mut String,
) {
    out_buf.clear();
    let mut first = true;

    let mut push = |val: u64| {
        use std::fmt::Write;
        if !first {
            out_buf.push(' ');
        }
        write!(out_buf, "{:>7}", val).unwrap();
        first = false;
    };

    if flag_l {
        push(l);
    }
    if flag_w {
        push(w);
    }
    if flag_m {
        push(ch);
    }
    if flag_c {
        push(c);
    }

    if show_file {
        out_buf.push(' ');
        out_buf.push_str(file);
    }

    writeln!(writer, "{out_buf}").ok();
}

register_command!(WC_CMD, "wc", "lwc(m)", CommandFlags::BIN.bits(), wc_main);
