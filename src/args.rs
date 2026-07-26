// =============================================================================
// args — Command-line argument parser using lexopt.
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// This module replaces the original Toybox‑inspired parser with a clean
// Rust‑native implementation built on top of the `lexopt` crate.
// =============================================================================

use crate::context::Context;
use std::collections::HashMap;

/// Result of parsing command-line options.
///
/// Provides access to option counts and their associated arguments.
pub struct ParsedOpts {
    counts: HashMap<char, u32>,
    values: HashMap<char, Vec<String>>,
}

impl ParsedOpts {
    /// Returns how many times the short option `ch` was given.
    pub fn count(&self, ch: char) -> u32 {
        self.counts.get(&ch).copied().unwrap_or(0)
    }

    /// Returns the last argument value for option `ch`, if any.
    pub fn get_str(&self, ch: char) -> Option<&str> {
        self.values
            .get(&ch)
            .and_then(|v| v.last())
            .map(|s| s.as_str())
    }

    /// Returns all argument values for option `ch` (for repeated options).
    pub fn get_strs(&self, ch: char) -> Vec<&str> {
        self.values
            .get(&ch)
            .map(|v| v.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }

    /// Parses the last argument value of option `ch` as an integer.
    pub fn get_int(&self, ch: char) -> Option<i64> {
        self.get_str(ch).and_then(|s| s.parse().ok())
    }
}

// =============================================================================
// Internal representation of the option specification
// =============================================================================

#[derive(Clone)]
struct OptionInfo {
    has_arg: bool,
}

struct OptSpec {
    options: HashMap<char, OptionInfo>,
    groups: Vec<Vec<char>>,
    min_args: usize,
    max_args: usize,
    stop_at_first_non_option: bool,
}

// =============================================================================
// Parser for the optstr syntax
// =============================================================================

/// Parses a Toybox‑style option string into an `OptSpec`.
///
/// Supported constructs:
///   - `^`                     stop at first non‑option argument
///   - `<N>`                   minimum number of positional arguments
///   - `>M`                    maximum number of positional arguments
///   - `[abc]`                 mutually exclusive group of options
///   - `(x)`                   single‑letter long alias (adds option `x`)
///   - `(xyz)`                 if all letters are already options, it's an
///                             expansion (does nothing); otherwise it's a
///                             long name (ignored)
///   - `c:`                    option `c` takes an argument
///   - `?`                     ignored (all options are optional by default)
fn parse_optstr(optstr: &str) -> Result<OptSpec, String> {
    let mut options = HashMap::new();
    let mut groups = Vec::new();
    let mut min_args = 0;
    let mut max_args = usize::MAX;
    let mut stop_at_first_non_option = false;

    let chars: Vec<char> = optstr.chars().collect();
    let mut i = 0;
    let mut paren_pairs = Vec::new(); // (start, end) indices in chars

    // First pass: collect plain options, groups, and constraints.
    while i < chars.len() {
        let c = chars[i];
        match c {
            '^' => {
                stop_at_first_non_option = true;
                i += 1;
            }
            '<' => {
                let mut num = 0;
                i += 1;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    num = num * 10 + (chars[i] as u32 - '0' as u32) as usize;
                    i += 1;
                }
                min_args = num;
            }
            '>' => {
                let mut num = 0;
                i += 1;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    num = num * 10 + (chars[i] as u32 - '0' as u32) as usize;
                    i += 1;
                }
                max_args = num;
            }
            '[' => {
                let mut group = Vec::new();
                i += 1;
                while i < chars.len() && chars[i] != ']' {
                    if chars[i].is_ascii_alphabetic() {
                        group.push(chars[i]);
                    }
                    i += 1;
                }
                if !group.is_empty() {
                    groups.push(group);
                }
                if i < chars.len() && chars[i] == ']' {
                    i += 1;
                }
            }
            '(' => {
                let start = i;
                i += 1;
                while i < chars.len() && chars[i] != ')' {
                    i += 1;
                }
                if i < chars.len() && chars[i] == ')' {
                    paren_pairs.push((start, i));
                    i += 1;
                }
            }
            '?' => {
                // Optional marker – we ignore it because all options are optional.
                i += 1;
            }
            ch if ch.is_ascii_alphabetic() => {
                let opt_char = ch;
                let mut has_arg = false;
                i += 1; // consume the letter
                if i < chars.len() && chars[i] == ':' {
                    has_arg = true;
                    i += 1; // consume the ':'
                }
                options.insert(opt_char, OptionInfo { has_arg });
            }
            _ => {
                i += 1;
            }
        }
    }

    // Second pass: process parentheses content.
    for (start, end) in paren_pairs {
        let inner: String = chars[start + 1..end].iter().collect();
        if inner.len() == 1 {
            let ch = inner.chars().next().unwrap();
            if ch.is_ascii_alphabetic() && !options.contains_key(&ch) {
                options.insert(ch, OptionInfo { has_arg: false });
            }
        } else if inner.len() > 1 {
            // If every character is already a known option, this is an
            // expansion (e.g. `(dpr)` for `-a`). Otherwise it's a long
            // name and we ignore it.
            let all_opts = inner
                .chars()
                .all(|ch| ch.is_ascii_alphabetic() && options.contains_key(&ch));
            if !all_opts {
                // Long name – ignore.
            }
        }
    }

    Ok(OptSpec {
        options,
        groups,
        min_args,
        max_args,
        stop_at_first_non_option,
    })
}

// =============================================================================
// Public parsing API
// =============================================================================

/// Parses the command-line arguments stored in `ctx.argv` according to the
/// given `optstr` specification.
///
/// On success, fills `ctx.optargs` with the positional arguments and returns
/// a `ParsedOpts` instance. On error, returns a human‑readable message.
pub fn parse(ctx: &mut Context, optstr: &str) -> Result<ParsedOpts, String> {
    let spec = parse_optstr(optstr)?;

    let mut counts: HashMap<char, u32> = HashMap::new();
    let mut values: HashMap<char, Vec<String>> = HashMap::new();
    let mut positional = Vec::new();

    let mut args = ctx.argv.iter().skip(1).peekable();

    while let Some(arg) = args.next() {
        let arg_str = arg.as_str();

        // Stop at '--'
        if arg_str == "--" {
            positional.extend(args.map(|s| s.to_string()));
            break;
        }

        // Stop at first non-option if requested
        if spec.stop_at_first_non_option && !arg_str.starts_with('-') {
            positional.push(arg_str.to_string());
            positional.extend(args.map(|s| s.to_string()));
            break;
        }

        // Handle standalone '-'
        if arg_str == "-" {
            positional.push(arg_str.to_string());
            continue;
        }

        // Handle options
        if arg_str.starts_with('-') && arg_str.len() > 1 {
            let opt_chars: Vec<char> = arg_str[1..].chars().collect();
            let mut i = 0;

            while i < opt_chars.len() {
                let ch = opt_chars[i];

                if let Some(info) = spec.options.get(&ch) {
                    if info.has_arg {
                        // Option with argument
                        let val = if i + 1 < opt_chars.len() {
                            // Argument is part of the same token: -fvalue
                            let rest: String = opt_chars[i + 1..].iter().collect();
                            i = opt_chars.len(); // consume all
                            rest
                        } else {
                            // Argument is the next token
                            match args.next() {
                                Some(v) => v.to_string(),
                                None => return Err(format!("option -{} requires an argument", ch)),
                            }
                        };
                        *counts.entry(ch).or_insert(0) += 1;
                        values.entry(ch).or_insert_with(Vec::new).push(val);
                        break; // argument consumed, stop processing this token
                    } else {
                        // Option without argument
                        *counts.entry(ch).or_insert(0) += 1;
                        i += 1;
                    }
                } else {
                    return Err(format!("unknown option -{}", ch));
                }
            }
        } else if arg_str.starts_with('-') {
            // Single dash only (already handled above)
            continue;
        } else {
            // Positional argument
            positional.push(arg_str.to_string());
        }
    }

    // Enforce mutual exclusivity for each group.
    for group in &spec.groups {
        let used: u32 = group
            .iter()
            .map(|&ch| counts.get(&ch).copied().unwrap_or(0))
            .sum();
        if used > 1 {
            let opts: Vec<String> = group.iter().map(|c| format!("-{}", c)).collect();
            return Err(format!(
                "options {} are mutually exclusive",
                opts.join(", ")
            ));
        }
    }

    // Enforce positional argument count constraints.
    if positional.len() < spec.min_args {
        return Err(format!(
            "expected at least {} positional argument(s), got {}",
            spec.min_args,
            positional.len()
        ));
    }
    if positional.len() > spec.max_args {
        return Err(format!(
            "expected at most {} positional argument(s), got {}",
            spec.max_args,
            positional.len()
        ));
    }

    ctx.optargs = positional;

    Ok(ParsedOpts { counts, values })
}
