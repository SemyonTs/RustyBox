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
//   -E         Interpret patterns as extended regular expressions.
//   -F         Interpret patterns as fixed strings (literal match).
//   -P         Interpret patterns as Perl-compatible regular expressions.
//              Enables Unicode property escapes (\p{...}) and \x{NNNN}.
//   -i         Case-insensitive matching.
//   -v         Invert match: select non-matching lines.
//   -n         Prefix each output line with its 1-based line number.
//   -r, -R     Recursively search directories.
//   -c         Print only a count of matching lines per file.
//   -l         Print only the names of files containing at least one match.
//   -L         Print only the names of files containing no match.
//   -q         Quiet: suppress all normal output; exit immediately on first match.
//   -s         Suppress error messages for nonexistent or unreadable files.
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

/// A compiled pattern, either byte‑oriented (fast, no UTF‑8 validation)
/// or Unicode (required for -P and for -i with non‑ASCII patterns).
enum Pattern {
    Bytes(regex::bytes::Regex),
    Unicode(Regex),
}

/// Entry point for the `grep` builtin.
///
/// Exit codes:
///   0 — at least one match was found.
///   1 — no matches were found.
///   2 — an error occurred (syntax error in the pattern, etc.).
fn grep_main(ctx: &mut Context) -> u8 {
    // [EFP] enforces mutual exclusivity between -E, -F, and -P.
    let opts = match crate::args::parse(ctx, "EFivnrclLqwxe:f:hHsP[EFP]") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("grep: {e}");
            return 2;
        }
    };

    let flag_E = opts.count('E') > 0;
    let flag_F = opts.count('F') > 0;
    let flag_P = opts.count('P') > 0;
    let flag_i = opts.count('i') > 0;
    let flag_v = opts.count('v') > 0;
    let flag_n = opts.count('n') > 0;
    let flag_r = opts.count('r') > 0 || opts.count('R') > 0;
    let flag_c = opts.count('c') > 0;
    let flag_l = opts.count('l') > 0;
    let flag_L = opts.count('L') > 0;
    let flag_q = opts.count('q') > 0;
    let flag_s = opts.count('s') > 0;
    let flag_w = opts.count('w') > 0;
    let flag_x = opts.count('x') > 0;
    let flag_h = opts.count('h') > 0;
    let flag_H = opts.count('H') > 0;

    // Compiled patterns.
    let mut patterns: Vec<Pattern> = Vec::new();
    // POSIX: a null (empty) pattern matches every line.
    let mut has_empty_pattern = false;

    // Helper to compile a single pattern string with the current flags.
    let compile_pattern = |raw: &str| -> Result<Pattern, String> {
        // For -P we always produce a Unicode regex (required for \p{...}, etc.).
        if flag_P {
            let mut s = raw.to_string();
            if flag_w {
                s = format!(r"\b{}\b", s);
            }
            if flag_x {
                s = format!("^(?:{})$", s);
            }
            let re = if flag_i {
                Regex::new(&format!("(?i){}", s))
            } else {
                Regex::new(&s)
            };
            return re
                .map(Pattern::Unicode)
                .map_err(|e| format!("invalid regular expression '{}': {}", raw, e));
        }

        // Non‑P path: build the pattern string (BRE→ERE, escaping, etc.).
        let re_str = if flag_F {
            regex::escape(raw)
        } else {
            let mut s = if !flag_E {
                bre_to_ere(raw)
            } else {
                raw.to_string()
            };
            if flag_w {
                s = format!(r"\b{}\b", s);
            }
            if flag_x {
                s = format!("^(?:{})$", s);
            }
            s
        };

        // If -i is active and the pattern contains at least one non‑ASCII
        // character, we must use a Unicode regex to get correct case‑folding.
        let needs_unicode_for_case = flag_i && raw.chars().any(|c| !c.is_ascii());

        if needs_unicode_for_case {
            let re = if flag_i {
                Regex::new(&format!("(?i){}", re_str))
            } else {
                unreachable!();
            };
            return re
                .map(Pattern::Unicode)
                .map_err(|e| format!("invalid regular expression '{}': {}", raw, e));
        }

        // Fast path: byte‑oriented regex, no UTF‑8 validation during matching.
        let mut builder = regex::bytes::RegexBuilder::new(&re_str);
        if flag_i {
            builder.case_insensitive(true); // ASCII case‑insensitive is enough
        }
        let re = builder
            .build()
            .map_err(|e| format!("invalid regular expression '{}': {}", raw, e))?;
        Ok(Pattern::Bytes(re))
    };

    // Collect patterns from repeated -e options.
    for pat in opts.get_strs('e') {
        if pat.is_empty() {
            has_empty_pattern = true;
        } else {
            match compile_pattern(pat) {
                Ok(p) => patterns.push(p),
                Err(msg) => {
                    eprintln!("grep: {msg}");
                    return 2;
                }
            }
        }
    }

    // Collect patterns from -f file.
    if let Some(f) = opts.get_str('f') {
        let file = match File::open(f) {
            Ok(f) => f,
            Err(e) => {
                if !flag_q && !flag_s {
                    eprintln!("grep: cannot open '{}': {}", f, e);
                }
                return 2;
            }
        };
        let reader = BufReader::new(file);
        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(e) => {
                    if !flag_q && !flag_s {
                        eprintln!("grep: error reading '{}': {}", f, e);
                    }
                    return 2;
                }
            };
            if line.is_empty() {
                has_empty_pattern = true;
                continue;
            }
            match compile_pattern(&line) {
                Ok(p) => patterns.push(p),
                Err(msg) => {
                    eprintln!("grep: {msg}");
                    return 2;
                }
            }
        }
    }

    // If no -e/-f were given, the first positional argument is the pattern.
    let mut args_iter = ctx.optargs.iter().map(|s| s.as_str());
    if patterns.is_empty() && !has_empty_pattern {
        let pattern = match args_iter.next() {
            Some(p) => p,
            None => {
                eprintln!("grep: no pattern specified");
                return 2;
            }
        };
        match compile_pattern(pattern) {
            Ok(p) => patterns.push(p),
            Err(msg) => {
                eprintln!("grep: {msg}");
                return 2;
            }
        }
    }

    // The remaining arguments are file names (or "-" for stdin).
    let files: Vec<&str> = args_iter.collect();
    let files: Vec<&str> = if files.is_empty() { vec!["-"] } else { files };

    let multiple = files.len() > 1 || flag_r;
    let mut found_any = false;
    let mut has_error = false;

    let stdout = std::io::stdout();
    let mut writer = BufWriter::new(stdout.lock());

    for &file in &files {
        if flag_r && file != "-" {
            let result = grep_recursive(
                file,
                &patterns,
                has_empty_pattern,
                flag_v,
                flag_n,
                flag_c,
                flag_l,
                flag_L,
                flag_q,
                flag_s,
                flag_h,
                flag_H,
                multiple,
                &mut writer,
                &mut found_any,
                &mut has_error,
            );
            match result {
                Err(e) => {
                    if !flag_q && !flag_s {
                        eprintln!("grep: {e}");
                    }
                    has_error = true;
                }
                Ok(early_exit) => {
                    if early_exit {
                        return 0;
                    }
                }
            }
        } else {
            match grep_file(
                file,
                &patterns,
                has_empty_pattern,
                flag_v,
                flag_n,
                flag_c,
                flag_l,
                flag_L,
                flag_q,
                flag_s,
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
                    if !flag_q && !flag_s {
                        eprintln!("grep: {e}");
                    }
                    has_error = true;
                }
            }

            if flag_q && found_any {
                return 0;
            }
        }
    }

    writer.flush().ok();

    if has_error {
        return 2;
    }

    if flag_q {
        return if found_any { 0 } else { 1 };
    }

    if found_any { 0 } else { 1 }
}

/// Recursively walk `dir`, processing each regular file as it is discovered.
///
/// Returns `Ok(true)` if the caller should exit early (e.g. `-q` found a match).
fn grep_recursive(
    dir: &str,
    patterns: &[Pattern],
    has_empty_pattern: bool,
    flag_v: bool,
    flag_n: bool,
    flag_c: bool,
    flag_l: bool,
    flag_L: bool,
    flag_q: bool,
    flag_s: bool,
    flag_h: bool,
    flag_H: bool,
    multiple: bool,
    writer: &mut BufWriter<std::io::StdoutLock>,
    found_any: &mut bool,
    has_error: &mut bool,
) -> Result<bool, String> {
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) => {
            if flag_s {
                return Ok(false);
            }
            return Err(format!("'{dir}': {e}"));
        }
    };

    for item in rd {
        let entry = match item {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        if meta.is_dir() {
            let sub_path = path.to_str().unwrap_or_default();
            let early = grep_recursive(
                sub_path,
                patterns,
                has_empty_pattern,
                flag_v,
                flag_n,
                flag_c,
                flag_l,
                flag_L,
                flag_q,
                flag_s,
                flag_h,
                flag_H,
                multiple,
                writer,
                found_any,
                has_error,
            )?;
            if early {
                return Ok(true);
            }
        } else if meta.is_file() {
            let file_path = path.to_str().unwrap_or_default();
            match grep_file(
                file_path,
                patterns,
                has_empty_pattern,
                flag_v,
                flag_n,
                flag_c,
                flag_l,
                flag_L,
                flag_q,
                flag_s,
                flag_h,
                flag_H,
                multiple,
                writer,
            ) {
                Ok(found) => {
                    if found {
                        *found_any = true;
                    }
                }
                Err(e) => {
                    if !flag_q && !flag_s {
                        eprintln!("grep: {e}");
                    }
                    *has_error = true;
                }
            }

            if flag_q && *found_any {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

/// Search a single file (or stdin when `file == "-"`) and print matches
/// according to the active flags.
///
/// This function reads raw bytes and avoids UTF‑8 validation unless a
/// Unicode pattern forces it.  For the common case (ASCII patterns,
/// no `-P`, no `-i` with non‑ASCII) no validation is performed at all.
///
/// Returns `Ok(true)` if at least one line matched.
fn grep_file(
    file: &str,
    patterns: &[Pattern],
    has_empty_pattern: bool,
    flag_v: bool,
    flag_n: bool,
    flag_c: bool,
    flag_l: bool,
    flag_L: bool,
    flag_q: bool,
    flag_s: bool,
    flag_h: bool,
    flag_H: bool,
    multiple: bool,
    writer: &mut BufWriter<std::io::StdoutLock>,
) -> Result<bool, String> {
    let mut reader: Box<dyn BufRead> = if file == "-" {
        Box::new(std::io::stdin().lock())
    } else {
        match File::open(file) {
            Ok(f) => Box::new(BufReader::new(f)),
            Err(e) => {
                if flag_s {
                    return Ok(false);
                }
                return Err(format!("'{file}': {e}"));
            }
        }
    };

    let mut count = 0u64;
    let mut found = false;
    let show_name = (multiple && !flag_h) || flag_H;

    // Reusable byte buffer – avoids allocating for every line.
    let mut buf: Vec<u8> = Vec::with_capacity(256);
    let mut line_number = 0usize;

    loop {
        buf.clear();
        let n = reader
            .read_until(b'\n', &mut buf)
            .map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        line_number += 1;

        // Strip the trailing newline for matching.
        let line_bytes = if buf.ends_with(b"\n") {
            &buf[..buf.len() - 1]
        } else {
            &buf[..]
        };

        // Determine whether the line matches any pattern.
        let matched = if has_empty_pattern {
            true
        } else {
            patterns.iter().any(|pat| match pat {
                Pattern::Bytes(re) => re.is_match(line_bytes),
                Pattern::Unicode(re) => {
                    // Only when a Unicode pattern is present do we pay
                    // the cost of UTF‑8 validation.  Invalid sequences
                    // are treated as non‑matching.
                    std::str::from_utf8(line_bytes)
                        .map(|s| re.is_match(s))
                        .unwrap_or(false)
                }
            })
        };

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

            if flag_L {
                continue;
            }

            if flag_c {
                continue;
            }

            // Print prefix and the raw line bytes.
            if show_name || flag_n {
                if show_name {
                    write!(writer, "{}:", file).map_err(|e| e.to_string())?;
                }
                if flag_n {
                    write!(writer, "{}:", line_number).map_err(|e| e.to_string())?;
                }
                writer.write_all(line_bytes).map_err(|e| e.to_string())?;
                writeln!(writer).map_err(|e| e.to_string())?;
            } else {
                writer.write_all(line_bytes).map_err(|e| e.to_string())?;
                writeln!(writer).map_err(|e| e.to_string())?;
            }
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

    // -L: print filenames that contain no matching lines.
    if flag_L && !found {
        writeln!(writer, "{}", file).map_err(|e| e.to_string())?;
    }

    Ok(found)
}

/// Simplified conversion from POSIX Basic Regular Expression (BRE) syntax
/// to Extended Regular Expression (ERE) syntax used by the `regex` crate.
///
/// This handles the most common differences: backslash‑escaped `?+{|}()`
/// become the corresponding ERE metacharacters, and unescaped occurrences
/// of those characters are backslash‑escaped so they are treated as literals.
fn bre_to_ere(bre: &str) -> String {
    let mut result = String::with_capacity(bre.len());
    let mut chars = bre.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(&next) = chars.peek() {
                    match next {
                        '?' | '+' | '{' | '}' | '|' | '(' | ')' => {
                            // In BRE: \?, \+, \{, \}, \|, \(, \) have special meaning.
                            // In ERE the same meaning is conveyed by the unescaped character.
                            result.push(next);
                            chars.next();
                        }
                        _ => {
                            // Other backslash sequences (e.g., \., \*, \\, \1) are kept as-is.
                            result.push('\\');
                            result.push(next);
                            chars.next();
                        }
                    }
                } else {
                    // Trailing backslash – pass through (invalid BRE, but let regex crate catch it).
                    result.push('\\');
                }
            }
            // In BRE the characters ?+{}|() are ordinary literals.
            // In ERE they are metacharacters, so they must be escaped.
            '?' | '+' | '{' | '}' | '|' | '(' | ')' => {
                result.push('\\');
                result.push(c);
            }
            _ => {
                result.push(c);
            }
        }
    }

    result
}

register_command!(
    GREP_CMD,
    "grep",
    "EFivnrclLqwxe:f:hHsP[EFP]",
    CommandFlags::BIN.bits(),
    grep_main,
    description = "Search for lines matching a regular expression",
    help = r#"\
OPTIONS:    
-E         Interpret patterns as extended regular expressions.
-F         Interpret patterns as fixed strings (literal match).
-P         Interpret patterns as Perl-compatible regular expressions.
           Enables Unicode property escapes (\p{...}) and \x{NNNN}.
-i         Case-insensitive matching.
-v         Invert match: select non-matching lines.
-n         Prefix each output line with its 1-based line number.
-r, -R     Recursively search directories.
-c         Print only a count of matching lines per file.
-l         Print only the names of files containing at least one match.
-L         Print only the names of files containing no match.
-q         Quiet: suppress all normal output; exit immediately on first match.
-s         Suppress error messages for nonexistent or unreadable files.
-w         Match whole words only.
-x         Match whole lines only (anchored at both ends).
-e PATTERN Use PATTERN as the pattern (may be repeated).
-f FILE    Read patterns from FILE, one per line.
-h         Suppress the filename prefix in output.
-H         Force the filename prefix in output."#
);
