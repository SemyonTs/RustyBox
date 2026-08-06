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

    // Compile patterns directly into Regex objects.
    let mut regexes: Vec<Regex> = Vec::new();
    // POSIX: a null (empty) pattern matches every line.
    let mut has_empty_pattern = false;

    // Helper to compile a pattern string with flags applied.
    let compile_pattern = |raw: &str| -> Result<Regex, String> {
        let re_str = if flag_F {
            regex::escape(raw)
        } else if flag_P {
            // -P mode: pass pattern directly to regex crate.
            // The regex crate natively supports PCRE-like syntax including
            // \x{NNNN}, \p{Cyrillic}, \d, \w, etc.
            let mut s = raw.to_string();
            if flag_w {
                s = format!(r"\b{}\b", s);
            }
            if flag_x {
                s = format!("^(?:{})$", s);
            }
            s
        } else {
            let mut s = raw.to_string();
            // When neither -E nor -F nor -P is given, treat the pattern as BRE.
            if !flag_E {
                s = bre_to_ere(&s);
            }
            if flag_w {
                s = format!(r"\b{}\b", s);
            }
            if flag_x {
                s = format!("^{}$", s);
            }
            s
        };

        let re = if flag_i {
            Regex::new(&format!("(?i){}", re_str))
        } else {
            Regex::new(&re_str)
        };

        re.map_err(|e| format!("invalid regular expression '{}': {}", raw, e))
    };

    // Collect patterns from repeated -e options.
    for pat in opts.get_strs('e') {
        if pat.is_empty() {
            has_empty_pattern = true;
        } else {
            match compile_pattern(pat) {
                Ok(re) => regexes.push(re),
                Err(msg) => {
                    eprintln!("grep: {msg}");
                    return 2;
                }
            }
        }
    }

    // Collect patterns from -f file (read and compile on the fly).
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
                // Empty line in a pattern file represents a null pattern.
                has_empty_pattern = true;
                continue;
            }
            match compile_pattern(&line) {
                Ok(re) => regexes.push(re),
                Err(msg) => {
                    eprintln!("grep: {msg}");
                    return 2;
                }
            }
        }
    }

    // If no -e/-f were given, the first positional argument is the pattern.
    let mut args_iter = ctx.optargs.iter().map(|s| s.as_str());
    if regexes.is_empty() && !has_empty_pattern {
        let pattern = match args_iter.next() {
            Some(p) => p,
            None => {
                eprintln!("grep: no pattern specified");
                return 2;
            }
        };
        match compile_pattern(pattern) {
            Ok(re) => regexes.push(re),
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

    // Use a buffered stdout lock for all output.
    let stdout = std::io::stdout();
    let mut writer = BufWriter::new(stdout.lock());

    for &file in &files {
        if flag_r && file != "-" {
            // Recursive mode: walk the directory tree lazily, processing each
            // file as it is discovered — no upfront collection into a Vec.
            let result = grep_recursive(
                file,
                &regexes,
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
            // Single file or stdin.
            match grep_file(
                file,
                &regexes,
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

    // Flush writer (important!)
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
    regexes: &[Regex],
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
            // Recurse into subdirectory.
            let sub_path = path.to_str().unwrap_or_default();
            let early = grep_recursive(
                sub_path,
                regexes,
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
                regexes,
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
        // Symlinks and other special file types are silently skipped.
    }

    Ok(false)
}

/// Search a single file (or stdin when `file == "-"`) and print matches
/// according to the active flags.
///
/// Returns `Ok(true)` if at least one line matched.
fn grep_file(
    file: &str,
    regexes: &[Regex],
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

    // Reusable line buffer – avoids allocating a new String for every line.
    let mut line_buf = String::with_capacity(256);
    let mut line_number = 0usize;

    loop {
        line_buf.clear();
        let n = reader.read_line(&mut line_buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        line_number += 1;

        // Remove trailing newline for accurate output if needed.
        let line = if line_buf.ends_with('\n') {
            &line_buf[..line_buf.len() - 1]
        } else {
            &line_buf[..]
        };

        let matched = has_empty_pattern || regexes.iter().any(|r| r.is_match(line));
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

            // When -L is active, suppress normal output; the filename will be
            // printed only if no match was found throughout the entire file.
            if flag_L {
                continue;
            }

            if flag_c {
                continue;
            }

            // Build prefix efficiently without allocating a new String
            // for the common case where no prefix is needed.
            if show_name || flag_n {
                if show_name {
                    write!(writer, "{}:", file).map_err(|e| e.to_string())?;
                }
                if flag_n {
                    write!(writer, "{}:", line_number).map_err(|e| e.to_string())?;
                }
            }
            writeln!(writer, "{}", line).map_err(|e| e.to_string())?;
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
