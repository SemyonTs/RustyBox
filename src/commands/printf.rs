// =============================================================================
// printf — Format and print data.
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Implementation inspired by Toybox (https://landley.net/toybox/)
// Toybox is copyrighted by Rob Landley, see NOTICE file for license details.
//
// Supported format specifiers:
//   %s, %d, %i, %u, %x, %X, %o, %f, %F, %e, %E, %g, %G, %c, %%, %b, %q.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;

/// Entry point for the `printf` builtin.
///
/// The first positional argument is the format string; subsequent arguments
/// supply values for the conversion specifiers.
fn printf_main(ctx: &mut Context) -> u8 {
    let args: Vec<String> = ctx.optargs.clone();
    if args.is_empty() {
        eprintln!("printf: missing format string");
        return 1;
    }

    let format = &args[0];
    let values = &args[1..];

    match format_string(format, values) {
        Ok(output) => {
            print!("{}", output);
            0
        }
        Err(e) => {
            eprintln!("printf: {}", e);
            1
        }
    }
}

/// Expand a `printf`-style format string with the supplied argument values.
///
/// Returns the fully rendered string or a description of the first
/// formatting error encountered.
fn format_string(fmt: &str, values: &[String]) -> Result<String, String> {
    let mut result = String::new();
    let chars: Vec<char> = fmt.chars().collect();
    let mut i = 0;
    let mut arg_idx = 0;

    while i < chars.len() {
        if chars[i] == '%' && i + 1 < chars.len() {
            i += 1;

            // Consume optional flags (subset: #, 0, -, space, +).
            while i < chars.len() && "#0- +".contains(chars[i]) {
                i += 1;
            }

            // Optional minimum field width.
            let mut width = 0;
            while i < chars.len() && chars[i].is_ascii_digit() {
                width = width * 10 + chars[i].to_digit(10).unwrap();
                i += 1;
            }

            // Optional precision.
            let mut prec: Option<usize> = None;
            if i < chars.len() && chars[i] == '.' {
                i += 1;
                let mut p = 0;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    p = p * 10 + chars[i].to_digit(10).unwrap();
                    i += 1;
                }
                prec = Some(p as usize);
            }

            if i >= chars.len() {
                return Err("unterminated % specifier".to_string());
            }

            let spec = chars[i];
            i += 1;

            let val = values.get(arg_idx).cloned().unwrap_or_default();
            arg_idx += 1;

            let formatted = format_arg(spec, &val, width as usize, prec)?;
            result.push_str(&formatted);
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    Ok(result)
}

/// Render a single argument according to the conversion specifier.
fn format_arg(
    spec: char,
    val: &str,
    width: usize,
    prec: Option<usize>,
) -> Result<String, String> {
    let s = match spec {
        '%' => "%".to_string(),

        's' => {
            if let Some(p) = prec {
                val.chars().take(p).collect()
            } else {
                val.to_string()
            }
        }

        'd' | 'i' => {
            let n: i64 = val
                .trim()
                .parse()
                .map_err(|_| format!("'{}' is not a number", val))?;
            n.to_string()
        }

        'u' => {
            let n: u64 = val
                .trim()
                .parse()
                .map_err(|_| format!("'{}' is not a number", val))?;
            n.to_string()
        }

        'x' => {
            let n: i64 = val
                .trim()
                .parse()
                .map_err(|_| format!("'{}' is not a number", val))?;
            format!("{:x}", n)
        }

        'X' => {
            let n: i64 = val
                .trim()
                .parse()
                .map_err(|_| format!("'{}' is not a number", val))?;
            format!("{:X}", n)
        }

        'o' => {
            let n: i64 = val
                .trim()
                .parse()
                .map_err(|_| format!("'{}' is not a number", val))?;
            format!("{:o}", n)
        }

        'f' | 'F' | 'e' | 'E' | 'g' | 'G' => {
            let n: f64 = val
                .trim()
                .parse()
                .map_err(|_| format!("'{}' is not a number", val))?;
            if let Some(p) = prec {
                format!("{:.*}", p, n)
            } else {
                n.to_string()
            }
        }

        'c' => val.chars().next().map(|c| c.to_string()).unwrap_or_default(),

        'b' => interpret_escapes(val),

        'q' => shell_quote(val),

        _ => return Err(format!("unknown specifier %{}", spec)),
    };

    // Apply minimum field width (right-aligned).
    if width > s.len() {
        Ok(format!("{:>width$}", s, width = width))
    } else {
        Ok(s)
    }
}

/// Map a backslash-escaped character to its literal value.
fn interpret_escape(c: char) -> char {
    match c {
        'n' => '\n',
        't' => '\t',
        'r' => '\r',
        'b' => '\x08',
        'f' => '\x0c',
        'v' => '\x0b',
        '0' => '\0',
        '\\' => '\\',
        other => other,
    }
}

/// Expand C-style backslash escapes in a string (for the `%b` specifier).
fn interpret_escapes(s: &str) -> String {
    let mut result = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            result.push(interpret_escape(chars[i + 1]));
            i += 2;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

/// Wrap a string in single quotes with interior quotes escaped for shell
/// consumption (the `%q` specifier).
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

register_command!(
    PRINTF_CMD,
    "printf",
    "",
    CommandFlags::BIN.bits(),
    printf_main
);