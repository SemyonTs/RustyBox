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
    if ctx.optargs.is_empty() {
        eprintln!("printf: missing format string");
        return 1;
    }

    let format = &ctx.optargs[0];
    let values = &ctx.optargs[1..];

    match format_string(format, values) {
        Ok(output) => {
            print!("{output}");
            0
        }
        Err(e) => {
            eprintln!("printf: {e}");
            1
        }
    }
}

/// Expand a `printf`-style format string with the supplied argument values.
///
/// Returns the fully rendered string or a description of the first
/// formatting error encountered.
fn format_string(fmt: &str, values: &[String]) -> Result<String, String> {
    let mut result = String::with_capacity(fmt.len() + values.len() * 16);
    let bytes = fmt.as_bytes();
    let mut i = 0;
    let mut arg_idx = 0;

    while i < bytes.len() {
        if bytes[i] == b'%' && i + 1 < bytes.len() {
            i += 1;

            // Consume optional flags (subset: #, 0, -, space, +).
            while i < bytes.len() && b"#0- +".contains(&bytes[i]) {
                i += 1;
            }

            // Optional minimum field width.
            let mut width = 0u32;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                width = width * 10 + (bytes[i] - b'0') as u32;
                i += 1;
            }

            // Optional precision.
            let mut prec: Option<usize> = None;
            if i < bytes.len() && bytes[i] == b'.' {
                i += 1;
                let mut p = 0u32;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    p = p * 10 + (bytes[i] - b'0') as u32;
                    i += 1;
                }
                prec = Some(p as usize);
            }

            if i >= bytes.len() {
                return Err("unterminated % specifier".to_string());
            }

            let spec = bytes[i] as char;
            i += 1;

            // %% does not consume an argument.
            if spec == '%' {
                result.push('%');
                continue;
            }

            let val = if arg_idx < values.len() {
                &values[arg_idx]
            } else {
                ""
            };
            arg_idx += 1;

            let formatted = format_arg(spec, val, width as usize, prec)?;
            result.push_str(&formatted);
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }

    Ok(result)
}

/// Render a single argument according to the conversion specifier.
fn format_arg(spec: char, val: &str, width: usize, prec: Option<usize>) -> Result<String, String> {
    let s = match spec {
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

        'c' => {
            // %c with empty argument prints nothing (not even NUL).
            val.chars()
                .next()
                .map(|c| c.to_string())
                .unwrap_or_default()
        }

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

/// Expand C-style backslash escapes in a string (for the `%b` specifier).
fn interpret_escapes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 1;
            result.push(match bytes[i] {
                b'n' => '\n',
                b't' => '\t',
                b'r' => '\r',
                b'b' => '\x08',
                b'f' => '\x0c',
                b'v' => '\x0b',
                b'0' => '\0',
                b'\\' => '\\',
                other => other as char,
            });
        } else {
            result.push(bytes[i] as char);
        }
        i += 1;
    }

    result
}

/// Wrap a string in single quotes with interior quotes escaped for shell
/// consumption (the `%q` specifier).
fn shell_quote(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 8);
    result.push('\'');
    for &b in s.as_bytes() {
        if b == b'\'' {
            result.push_str("'\\''");
        } else {
            result.push(b as char);
        }
    }
    result.push('\'');
    result
}

register_command!(
    PRINTF_CMD,
    "printf",
    "",
    CommandFlags::BIN.bits(),
    printf_main
);
