// =============================================================================
// args — Command-line argument parser (analogous to toybox/lib/args.c).
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Implementation inspired by Toybox (https://landley.net/toybox/)
// Toybox is copyrighted by Rob Landley, see NOTICE file for license details.
//
// Supported Toybox `get_optflags` syntax subset:
//   - Short options: `-a`, `-abc` (concatenated).
//   - Long options:  `(longname)` → `--longname`.
//   - Option-argument type suffixes:
//       `:`  string (stored in a `HashMap<char, String>`).
//       `#`  integer (stored in a `HashMap<char, i64>`).
//   - Prefixes at the start of the option string:
//       `<N`  minimum number of positional arguments.
//       `>N`  maximum number of positional arguments.
//       `^`   stop parsing after the first non-option argument.
//   - `--` terminates option parsing.
//
// Each option corresponds to a bit in `Context.optflags` in left-to-right
// order (rightmost option = bit 0, matching Toybox's convention).
// =============================================================================

use crate::context::Context;
use std::collections::HashMap;

/// Result of option parsing, made available to command implementations.
#[derive(Default)]
pub struct ParsedOpts {
    /// String values keyed by option character (last occurrence wins).
    pub strings: HashMap<char, String>,
    /// All string values for options that appear multiple times.
    pub multi_strings: HashMap<char, Vec<String>>,
    /// Integer values keyed by option character.
    pub ints: HashMap<char, i64>,
    /// Occurrence counters keyed by option character.
    pub counts: HashMap<char, u32>,
}

impl ParsedOpts {
    /// Return the last string argument associated with option `c`.
    pub fn get_str(&self, c: char) -> Option<&str> {
        self.strings.get(&c).map(|s| s.as_str())
    }

    /// Return all string arguments associated with option `c`.
    pub fn get_strs(&self, c: char) -> &[String] {
        self.multi_strings
            .get(&c)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Return the integer argument associated with option `c`, if any.
    pub fn get_int(&self, c: char) -> Option<i64> {
        self.ints.get(&c).copied()
    }

    /// Return the number of times option `c` appeared (0 if absent).
    pub fn count(&self, c: char) -> u32 {
        self.counts.get(&c).copied().unwrap_or(0)
    }

    /// Convenience alias for `count(c) > 0`.
    pub fn has(&self, c: char) -> bool {
        self.count(c) > 0
    }
}

/// Internal description of a single parsed option.
struct OptSpec {
    ch: char,
    takes_arg: bool,
    is_int: bool,
}

/// Parse the option string `optstr` against the argument vector in `ctx`,
/// populating `ctx.optflags` and `ctx.optargs` and returning a `ParsedOpts`
/// value with any option arguments.
pub fn parse(ctx: &mut Context, optstr: &str) -> Result<ParsedOpts, String> {
    let mut specs: Vec<OptSpec> = Vec::new();
    let mut long_map: HashMap<String, usize> = HashMap::new();

    // --- Prefixes at the start of the option string ---
    let mut min_args: usize = 0;
    let mut max_args: usize = usize::MAX;
    let mut stop_at_first_nonopt = false;

    let bytes = optstr.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'<' => {
                let (n, ni) = read_num(optstr, i + 1)?;
                min_args = n as usize;
                i = ni;
            }
            b'>' => {
                let (n, ni) = read_num(optstr, i + 1)?;
                max_args = n as usize;
                i = ni;
            }
            b'^' => {
                stop_at_first_nonopt = true;
                i += 1;
            }
            // Skip grouping/mutual-exclusion syntax (square brackets and parentheses
            // with letters).  They are accepted but not enforced.
            b'[' | b'(' => {
                while i < bytes.len() && bytes[i] != b']' && bytes[i] != b')' {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1; // skip closing bracket/paren
                }
            }
            c if c.is_ascii_alphanumeric() => break,
            _ => i += 1,
        }
    }

    // --- Parse option descriptors and long-option names ---
    let mut idx = i;
    while idx < optstr.len() {
        let c = optstr.as_bytes()[idx];

        if c == b'(' {
            let end = optstr[idx..]
                .find(')')
                .ok_or("unclosed parenthesis in opt string")?;
            let name = optstr[idx + 1..idx + end].to_string();
            long_map.insert(name, specs.len());
            specs.push(OptSpec {
                ch: '\0',
                takes_arg: false,
                is_int: false,
            });
            idx += end + 1;
            continue;
        }

        if !c.is_ascii_alphanumeric() {
            idx += 1;
            continue;
        }

        let ch = c as char;
        idx += 1;

        let mut takes_arg = false;
        let mut is_int = false;

        while idx < optstr.len() {
            match optstr.as_bytes()[idx] {
                b':' => {
                    takes_arg = true;
                    idx += 1;
                }
                b'#' => {
                    takes_arg = true;
                    is_int = true;
                    idx += 1;
                }
                _ => break,
            }
        }

        specs.push(OptSpec {
            ch,
            takes_arg,
            is_int,
        });
    }

    // Rightmost option = bit 0 (Toybox convention).
    let bit_of = |pos: usize| -> u32 { (specs.len() - 1 - pos) as u32 };

    let mut parsed = ParsedOpts::default();
    let argv = &ctx.argv;
    let mut i = 1; // skip program name
    let mut done = false;

    // Reuse ctx.optargs instead of allocating a new Vec.
    ctx.optargs.clear();

    while i < argv.len() {
        let arg = &argv[i];

        if done {
            ctx.optargs.push(arg.clone());
            i += 1;
            continue;
        }

        if arg == "--" {
            done = true;
            i += 1;
            continue;
        }

        // Long option.
        if arg.starts_with("--") && arg.len() > 2 {
            let name = &arg[2..];
            let (lname, largs) = match name.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (name, None),
            };

            let pos = *long_map
                .get(lname)
                .ok_or_else(|| format!("unknown option --{}", lname))?;
            let spec = &specs[pos];
            ctx.optflags |= 1u64 << bit_of(pos);

            if spec.takes_arg {
                let val = match largs {
                    Some(v) => v,
                    None => {
                        i += 1;
                        if i >= argv.len() {
                            return Err(format!("option --{} requires an argument", lname));
                        }
                        argv[i].clone()
                    }
                };
                store_arg(&mut parsed, spec, val)?;
            } else {
                *parsed.counts.entry('\0').or_insert(0) += 1;
            }
            i += 1;
        }
        // Short option(s).
        else if arg.starts_with('-') && arg.len() > 1 {
            let mut chars = arg[1..].chars().peekable();
            // Process each character in the cluster.
            loop {
                let ch = match chars.next() {
                    Some(c) => c,
                    None => break,
                };
                let pos = specs
                    .iter()
                    .position(|s| s.ch == ch)
                    .ok_or_else(|| format!("unknown option -{}", ch))?;
                let spec = &specs[pos];
                ctx.optflags |= 1u64 << bit_of(pos);

                if spec.takes_arg {
                    let rest: String = chars.collect();
                    let val = if !rest.is_empty() {
                        rest
                    } else {
                        i += 1;
                        if i >= argv.len() {
                            return Err(format!("option -{} requires an argument", ch));
                        }
                        argv[i].clone()
                    };
                    chars = "".chars().peekable(); // exhausted
                    store_arg(&mut parsed, spec, val)?;
                } else {
                    *parsed.counts.entry(ch).or_insert(0) += 1;
                }
            }
            i += 1;
        }
        // Positional argument.
        else {
            ctx.optargs.push(arg.clone());
            i += 1;
            if stop_at_first_nonopt {
                done = true;
            }
        }
    }

    // Enforce positional-argument count constraints.
    if ctx.optargs.len() < min_args {
        return Err(format!(
            "not enough arguments: need at least {}, got {}",
            min_args,
            ctx.optargs.len()
        ));
    }
    if ctx.optargs.len() > max_args {
        return Err(format!(
            "too many arguments: maximum {}, got {}",
            max_args,
            ctx.optargs.len()
        ));
    }

    Ok(parsed)
}

/// Store an option argument value in the appropriate map inside `ParsedOpts`.
/// Supports repeated options by accumulating into `multi_strings`.
fn store_arg(parsed: &mut ParsedOpts, spec: &OptSpec, val: String) -> Result<(), String> {
    if spec.is_int {
        let n: i64 = val
            .parse()
            .map_err(|_| format!("expected a number, got '{}'", val))?;
        parsed.ints.insert(spec.ch, n);
    } else {
        // Always update the last value in `strings`.
        parsed.strings.insert(spec.ch, val.clone());
        // Append to the list of all values for this option.
        parsed
            .multi_strings
            .entry(spec.ch)
            .or_insert_with(Vec::new)
            .push(val);
    }
    Ok(())
}

/// Read a decimal number from the option string starting at `start`.
fn read_num(s: &str, start: usize) -> Result<(i64, usize), String> {
    let rest = &s[start..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());

    if end == 0 {
        return Err("expected a number in option string".to_string());
    }

    let n: i64 = rest[..end].parse().map_err(|_| "invalid number")?;
    Ok((n, start + end))
}
