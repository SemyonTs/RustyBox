// =============================================================================
// sort — Sort, merge, or sequence check text files.
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
//   -b        Ignore leading blanks when determining sort keys.
//   -c        Check that input is sorted; do not output.
//   -C        Like -c but silent on disorder.
//   -d        Dictionary order: only blanks and alphanumerics are significant.
//   -f        Fold case: treat lowercase as uppercase equivalent.
//   -i        Ignore non-printable characters.
//   -k KEYDEF Define a sort key field.
//   -m        Merge already-sorted files.
//   -n        Numeric sort.
//   -o FILE   Write output to FILE instead of stdout.
//   -r        Reverse comparison result.
//   -t CHAR   Use CHAR as field separator.
//   -u        Unique: suppress duplicate lines based on sort keys.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::cmp::Ordering;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};

/// Parsed sort key specification from -k option.
#[derive(Clone)]
struct KeySpec {
    /// 1-based field start index.
    field_start: usize,
    /// Optional 1-based character offset within start field.
    char_start: Option<usize>,
    /// 1-based field end index (inclusive). None means last field.
    field_end: Option<usize>,
    /// Optional 1-based character offset within end field.
    char_end: Option<usize>,
    /// Per-key modifiers that override global flags.
    ignore_blanks: bool,
    numeric: bool,
    reverse: bool,
    fold_case: bool,
    dictionary: bool,
    ignore_nonprint: bool,
}

impl KeySpec {
    fn new() -> Self {
        Self {
            field_start: 1,
            char_start: None,
            field_end: None,
            char_end: None,
            ignore_blanks: false,
            numeric: false,
            reverse: false,
            fold_case: false,
            dictionary: false,
            ignore_nonprint: false,
        }
    }
}

/// Global sort configuration derived from command-line options.
struct SortConfig {
    ignore_blanks: bool,
    numeric: bool,
    reverse: bool,
    fold_case: bool,
    dictionary: bool,
    ignore_nonprint: bool,
    unique: bool,
    check: bool,
    check_silent: bool,
    merge: bool,
    separator: Option<char>,
    output_file: Option<String>,
    keys: Vec<KeySpec>,
}

/// Parse a -k keydef string into a KeySpec.
fn parse_keydef(s: &str, global: &SortConfig) -> Result<KeySpec, String> {
    let mut spec = KeySpec::new();

    // Split on comma for start,end
    let (start_part, end_part) = if let Some(pos) = s.find(',') {
        (&s[..pos], Some(&s[pos + 1..]))
    } else {
        (s, None)
    };

    // Parse field_start[.char_start][modifiers]
    let (start_nums, start_mods) = split_nums_and_mods(start_part);
    let (fs, cs) = parse_field_char(start_nums)?;
    spec.field_start = fs;
    spec.char_start = cs;

    // Parse field_end[.char_end][modifiers]
    if let Some(end_str) = end_part {
        let (end_nums, end_mods) = split_nums_and_mods(end_str);
        let (fe, ce) = parse_field_char(end_nums)?;
        spec.field_end = Some(fe);
        spec.char_end = ce;
        apply_modifiers(&mut spec, end_mods);
    }

    apply_modifiers(&mut spec, start_mods);

    // Inherit global flags for any modifier not explicitly set on this key.
    // Per POSIX: "If any modifier is attached to a field_start or to a
    // field_end, no option shall apply to either." We interpret this as:
    // if NO per-key modifiers are set at all, inherit globals.
    let has_per_key = spec.ignore_blanks
        || spec.numeric
        || spec.reverse
        || spec.fold_case
        || spec.dictionary
        || spec.ignore_nonprint;

    if !has_per_key {
        spec.ignore_blanks = global.ignore_blanks;
        spec.numeric = global.numeric;
        spec.reverse = global.reverse;
        spec.fold_case = global.fold_case;
        spec.dictionary = global.dictionary;
        spec.ignore_nonprint = global.ignore_nonprint;
    }

    Ok(spec)
}

/// Split "2.3bn" into ("2.3", "bn").
fn split_nums_and_mods(s: &str) -> (&str, &str) {
    let first_alpha = s.find(|c: char| c.is_ascii_alphabetic());
    match first_alpha {
        Some(pos) => (&s[..pos], &s[pos..]),
        None => (s, ""),
    }
}

/// Parse "2.3" into (field=2, char=Some(3)) or "2" into (field=2, char=None).
fn parse_field_char(s: &str) -> Result<(usize, Option<usize>), String> {
    if let Some(dot) = s.find('.') {
        let field: usize = s[..dot]
            .parse()
            .map_err(|_| format!("invalid field number '{}'", &s[..dot]))?;
        let ch: usize = s[dot + 1..]
            .parse()
            .map_err(|_| format!("invalid character position '{}'", &s[dot + 1..]))?;
        Ok((field, Some(ch)))
    } else {
        let field: usize = s
            .parse()
            .map_err(|_| format!("invalid field number '{}'", s))?;
        Ok((field, None))
    }
}

/// Apply modifier characters (b, d, f, i, n, r) to a KeySpec.
fn apply_modifiers(spec: &mut KeySpec, mods: &str) {
    for ch in mods.chars() {
        match ch {
            'b' => spec.ignore_blanks = true,
            'd' => spec.dictionary = true,
            'f' => spec.fold_case = true,
            'i' => spec.ignore_nonprint = true,
            'n' => spec.numeric = true,
            'r' => spec.reverse = true,
            _ => {} // Unknown modifiers are silently ignored per practice.
        }
    }
}

/// Extract a sort key substring from a line according to a KeySpec.
fn extract_key(line: &str, spec: &KeySpec, separator: Option<char>) -> String {
    let fields: Vec<&str> = match separator {
        Some(sep) => line.split(sep).collect(),
        None => {
            // Default: maximal sequences of non-blank chars separated by blanks.
            // Leading blanks are part of field 1 when no -t is specified.
            let trimmed = line.trim_start();
            if trimmed.is_empty() {
                return String::new();
            }
            // Re-split respecting that leading blanks belong to field 1.
            let lead = line.len() - trimmed.len();
            let mut result = Vec::new();
            if lead > 0 {
                // Field 1 includes leading blanks + first non-blank word
                let rest = trimmed.split_whitespace().next().unwrap_or("");
                result.push(&line[..lead + rest.len()]);
                for w in trimmed.split_whitespace().skip(1) {
                    result.push(w);
                }
            } else {
                for w in trimmed.split_whitespace() {
                    result.push(w);
                }
            }
            result
        }
    };

    if spec.field_start == 0 || spec.field_start > fields.len() {
        return String::new();
    }

    let start_idx = spec.field_start - 1;
    let end_idx = spec.field_end.map(|e| e - 1).unwrap_or(fields.len() - 1);
    let end_idx = end_idx.min(fields.len() - 1);

    if start_idx > end_idx {
        return String::new();
    }

    // Build the key from fields[start_idx..=end_idx]
    let mut key_parts: Vec<&str> = Vec::new();
    for i in start_idx..=end_idx {
        key_parts.push(fields[i]);
    }

    let raw_key = if separator.is_some() {
        let sep_str = separator.unwrap().to_string();
        key_parts.join(&sep_str)
    } else {
        key_parts.join(" ")
    };

    // Apply character offsets
    let key = if spec.char_start.is_some() || spec.char_end.is_some() {
        let chars: Vec<char> = raw_key.chars().collect();
        let cs = spec.char_start.map(|c| c - 1).unwrap_or(0);
        let ce = spec.char_end.unwrap_or(chars.len());
        let cs = cs.min(chars.len());
        let ce = ce.min(chars.len());
        chars[cs..ce].iter().collect()
    } else {
        raw_key
    };

    // Apply ignore_blanks: trim leading/trailing blanks from the extracted key
    if spec.ignore_blanks {
        key.trim().to_string()
    } else {
        key
    }
}

/// Compare two extracted key strings according to the KeySpec.
fn compare_keys(a: &str, b: &str, spec: &KeySpec) -> Ordering {
    let cmp = if spec.numeric {
        let na = a.trim().parse::<f64>().unwrap_or(0.0);
        let nb = b.trim().parse::<f64>().unwrap_or(0.0);
        na.partial_cmp(&nb).unwrap_or(Ordering::Equal)
    } else {
        let sa = if spec.fold_case {
            a.to_uppercase()
        } else {
            a.to_string()
        };
        let sb = if spec.fold_case {
            b.to_uppercase()
        } else {
            b.to_string()
        };

        let sa = if spec.dictionary {
            sa.chars()
                .filter(|c| c.is_ascii_alphanumeric() || c.is_ascii_whitespace())
                .collect::<String>()
        } else {
            sa
        };
        let sb = if spec.dictionary {
            sb.chars()
                .filter(|c| c.is_ascii_alphanumeric() || c.is_ascii_whitespace())
                .collect::<String>()
        } else {
            sb
        };

        let sa = if spec.ignore_nonprint {
            sa.chars()
                .filter(|c| !c.is_ascii_control())
                .collect::<String>()
        } else {
            sa
        };
        let sb = if spec.ignore_nonprint {
            sb.chars()
                .filter(|c| !c.is_ascii_control())
                .collect::<String>()
        } else {
            sb
        };

        sa.cmp(&sb)
    };

    if spec.reverse { cmp.reverse() } else { cmp }
}

/// Entry point for the `sort` builtin.
fn sort_main(ctx: &mut Context) -> u8 {
    // Full POSIX optstr for sort.
    let opts = match crate::args::parse(ctx, "bcCdfik:mno:rt:u") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("sort: {e}");
            return 2;
        }
    };

    let mut config = SortConfig {
        ignore_blanks: opts.count('b') > 0,
        numeric: opts.count('n') > 0,
        reverse: opts.count('r') > 0,
        fold_case: opts.count('f') > 0,
        dictionary: opts.count('d') > 0,
        ignore_nonprint: opts.count('i') > 0,
        unique: opts.count('u') > 0,
        check: opts.count('c') > 0,
        check_silent: opts.count('C') > 0,
        merge: opts.count('m') > 0,
        separator: opts.get_str('t').and_then(|s| s.chars().next()),
        output_file: opts.get_str('o').map(|s| s.to_string()),
        keys: Vec::new(),
    };

    // Parse all -k options.
    for kdef in opts.get_strs('k') {
        match parse_keydef(kdef, &config) {
            Ok(spec) => config.keys.push(spec),
            Err(e) => {
                eprintln!("sort: {e}");
                return 2;
            }
        }
    }

    // If no -k specified, default key is entire line with global flags.
    if config.keys.is_empty() {
        let mut default_key = KeySpec::new();
        default_key.ignore_blanks = config.ignore_blanks;
        default_key.numeric = config.numeric;
        default_key.reverse = config.reverse;
        default_key.fold_case = config.fold_case;
        default_key.dictionary = config.dictionary;
        default_key.ignore_nonprint = config.ignore_nonprint;
        config.keys.push(default_key);
    }

    // Read all input lines.
    let mut lines: Vec<String> = Vec::new();
    if ctx.optargs.is_empty() {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(l) => lines.push(l),
                Err(_) => break,
            }
        }
    } else {
        for file in &ctx.optargs {
            if file == "-" {
                let stdin = std::io::stdin();
                for line in stdin.lock().lines() {
                    match line {
                        Ok(l) => lines.push(l),
                        Err(_) => break,
                    }
                }
            } else {
                match File::open(file) {
                    Ok(f) => {
                        let reader = BufReader::new(f);
                        for line in reader.lines() {
                            match line {
                                Ok(l) => lines.push(l),
                                Err(e) => {
                                    eprintln!("sort: '{}': {}", file, e);
                                    return 2;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("sort: '{}': {}", file, e);
                        return 2;
                    }
                }
            }
        }
    }

    // Check mode: verify ordering without producing output.
    if config.check || config.check_silent {
        for i in 1..lines.len() {
            let prev = &lines[i - 1];
            let curr = &lines[i];
            let mut ordered = Ordering::Less;
            for key_spec in &config.keys {
                let ka = extract_key(prev, key_spec, config.separator);
                let kb = extract_key(curr, key_spec, config.separator);
                let cmp = compare_keys(&ka, &kb, key_spec);
                if cmp != Ordering::Equal {
                    ordered = cmp;
                    break;
                }
            }
            if config.unique && ordered == Ordering::Equal {
                if !config.check_silent {
                    eprintln!("sort: disorder at line {}", i + 1);
                }
                return 1;
            }
            if ordered == Ordering::Greater {
                if !config.check_silent {
                    eprintln!("sort: disorder at line {}", i + 1);
                }
                return 1;
            }
        }
        return 0;
    }

    // Sort using stable sort to preserve relative order of equal keys.
    let sep = config.separator;
    let keys = &config.keys;
    lines.sort_by(|a, b| {
        for key_spec in keys {
            let ka = extract_key(a, key_spec, sep);
            let kb = extract_key(b, key_spec, sep);
            let cmp = compare_keys(&ka, &kb, key_spec);
            if cmp != Ordering::Equal {
                return cmp;
            }
        }
        // Final byte-by-byte comparison for total ordering per POSIX.
        a.cmp(b)
    });

    // Remove duplicates if -u is specified.
    if config.unique {
        let sep = config.separator;
        let keys = &config.keys;
        lines.dedup_by(|a, b| {
            for key_spec in keys {
                let ka = extract_key(a, key_spec, sep);
                let kb = extract_key(b, key_spec, sep);
                let cmp = compare_keys(&ka, &kb, key_spec);
                if cmp != Ordering::Equal {
                    return false;
                }
            }
            true
        });
    }

    // Write output.
    let result: u8 = if let Some(ref outfile) = config.output_file {
        match File::create(outfile) {
            Ok(f) => {
                let mut writer = BufWriter::new(f);
                for line in &lines {
                    if writeln!(writer, "{}", line).is_err() {
                        return 2;
                    }
                }
                writer.flush().ok();
                0
            }
            Err(e) => {
                eprintln!("sort: '{}': {}", outfile, e);
                2
            }
        }
    } else {
        let stdout = std::io::stdout();
        let mut writer = BufWriter::new(stdout.lock());
        for line in &lines {
            if writeln!(writer, "{}", line).is_err() {
                return 2;
            }
        }
        writer.flush().ok();
        0
    };

    result
}

register_command!(
    SORT_CMD,
    "sort",
    "bcCdfik:mno:rt:u",
    CommandFlags::BIN.bits() | CommandFlags::USR.bits(),
    sort_main
);
