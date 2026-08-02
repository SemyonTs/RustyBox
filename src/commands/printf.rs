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

use crate::args;
use crate::context::Context;
use crate::flags::CommandFlags;
use std::io::Write;

/// Entry point for the `printf` builtin.
///
/// The first positional argument is the format string; subsequent arguments
/// supply values for the conversion specifiers.
fn printf_main(ctx: &mut Context) -> u8 {
    // Parse arguments using "^<1":
    //   ^ : stop parsing options at the first non-option argument
    //   <1: require at least one positional argument (the format string)
    if let Err(e) = args::parse(ctx, "^<1") {
        eprintln!("printf: {}", e);
        return 1;
    }

    let format = &ctx.optargs[0];
    let values = &ctx.optargs[1..];

    let (output, has_error) = format_string(format, values);

    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(&output);

    if has_error { 1 } else { 0 }
}

/// Expand a `printf`-style format string with the supplied argument values.
///
/// Returns the fully rendered byte string and a boolean indicating if any
/// formatting errors were encountered. The format operand is reused as often
/// as necessary to satisfy all argument operands per POSIX specification.
fn format_string(fmt: &str, values: &[String]) -> (Vec<u8>, bool) {
    let mut result = Vec::with_capacity(fmt.len() + values.len() * 16);
    let bytes = fmt.as_bytes();
    let mut arg_idx = 0;
    let mut has_error = false;

    loop {
        let mut i = 0;
        let mut pass_result = Vec::with_capacity(fmt.len());
        let mut consumed_args = 0;
        let mut abort = false;

        while i < bytes.len() {
            if bytes[i] == b'\\' && i + 1 < bytes.len() {
                i += 1;
                match bytes[i] {
                    b'a' => pass_result.push(0x07),
                    b'b' => pass_result.push(0x08),
                    b'f' => pass_result.push(0x0c),
                    b'n' => pass_result.push(b'\n'),
                    b'r' => pass_result.push(b'\r'),
                    b't' => pass_result.push(b'\t'),
                    b'v' => pass_result.push(0x0b),
                    b'\\' => pass_result.push(b'\\'),
                    b'0'..=b'7' => {
                        // \ddd: 1 to 3 octal digits
                        let mut octal = String::new();
                        let mut j = i;
                        while j < bytes.len()
                            && octal.len() < 3
                            && bytes[j] >= b'0'
                            && bytes[j] <= b'7'
                        {
                            octal.push(bytes[j] as char);
                            j += 1;
                        }
                        if octal.is_empty() {
                            pass_result.push(b'\\');
                            pass_result.push(bytes[i]);
                        } else {
                            let val = u8::from_str_radix(&octal, 8).unwrap_or(0);
                            pass_result.push(val);
                        }
                        i = j - 1;
                    }
                    other => {
                        // Unspecified escape: output backslash and character
                        pass_result.push(b'\\');
                        pass_result.push(other);
                    }
                }
                i += 1;
            } else if bytes[i] == b'%' && i + 1 < bytes.len() {
                i += 1;

                let mut zero = false;
                let mut left = false;

                // Consume optional flags (subset: #, 0, -, space, +).
                while i < bytes.len() && b"#0- +".contains(&bytes[i]) {
                    if bytes[i] == b'0' {
                        zero = true;
                    }
                    if bytes[i] == b'-' {
                        left = true;
                    }
                    i += 1;
                }

                // Optional minimum field width.
                let mut width = 0usize;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    width = width * 10 + (bytes[i] - b'0') as usize;
                    i += 1;
                }

                // Optional precision.
                let mut prec: Option<usize> = None;
                if i < bytes.len() && bytes[i] == b'.' {
                    i += 1;
                    let mut p = 0usize;
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        p = p * 10 + (bytes[i] - b'0') as usize;
                        i += 1;
                    }
                    prec = Some(p);
                }

                if i >= bytes.len() {
                    eprintln!("printf: unterminated % specifier");
                    has_error = true;
                    break;
                }

                let spec = bytes[i] as char;
                i += 1;

                // %% does not consume an argument.
                if spec == '%' {
                    pass_result.push(b'%');
                    continue;
                }

                let val = if arg_idx < values.len() {
                    &values[arg_idx]
                } else {
                    ""
                };
                arg_idx += 1;
                consumed_args += 1;

                let (formatted, b_abort) =
                    format_arg(spec, val, width, prec, zero, left, &mut has_error);
                pass_result.extend_from_slice(&formatted);
                if b_abort {
                    abort = true;
                    break;
                }
            } else {
                pass_result.push(bytes[i]);
                i += 1;
            }
        }

        result.extend_from_slice(&pass_result);

        // Break if \c was encountered, no conversion specifications consumed
        // arguments in this pass, or all arguments have been satisfied.
        if abort || consumed_args == 0 || arg_idx >= values.len() {
            break;
        }
    }

    (result, has_error)
}

/// Evaluate strings as unsuffixed C integer constants, with POSIX extensions
/// for leading quotes and signs.
fn parse_c_int(val: &str) -> Result<i64, String> {
    let s = val.trim();
    if s.is_empty() {
        return Ok(0);
    }

    let first = s.as_bytes()[0];

    // Handle character constants ('A' or "A")
    if first == b'\'' || first == b'"' {
        let rest = &s[1..];
        if let Some(c) = rest.chars().next() {
            return Ok(c as i64);
        } else {
            return Ok(0);
        }
    }

    let (sign, s) = if first == b'-' {
        (-1i64, &s[1..])
    } else if first == b'+' {
        (1i64, &s[1..])
    } else {
        (1i64, s)
    };

    let (radix, s) = if s.starts_with("0x") || s.starts_with("0X") {
        (16, &s[2..])
    } else if s.starts_with('0') && s.len() > 1 {
        (8, &s[1..])
    } else {
        (10, s)
    };

    let n =
        i64::from_str_radix(s, radix).map_err(|_| format!("'{}' expected numeric value", val))?;
    Ok(sign * n)
}

/// Pad formatted output to the specified field width.
fn pad_output(s_bytes: &[u8], width: usize, zero: bool, left: bool, is_num: bool) -> Vec<u8> {
    if width <= s_bytes.len() {
        return s_bytes.to_vec();
    }
    let pad_len = width - s_bytes.len();
    let mut res = Vec::with_capacity(width);
    let pad_char = if is_num && zero && !left { b'0' } else { b' ' };

    if !left {
        res.extend(std::iter::repeat(pad_char).take(pad_len));
    }
    res.extend_from_slice(s_bytes);
    if left {
        res.extend(std::iter::repeat(b' ').take(pad_len));
    }
    res
}

/// Render a single argument according to the conversion specifier.
fn format_arg(
    spec: char,
    val: &str,
    width: usize,
    prec: Option<usize>,
    zero: bool,
    left: bool,
    has_error: &mut bool,
) -> (Vec<u8>, bool) {
    let (s_bytes, b_abort, is_num) = match spec {
        's' => {
            let bytes = val.as_bytes();
            let s = if let Some(p) = prec {
                bytes[..p.min(bytes.len())].to_vec()
            } else {
                bytes.to_vec()
            };
            (s, false, false)
        }

        'd' | 'i' => match parse_c_int(val) {
            Ok(n) => (n.to_string().into_bytes(), false, true),
            Err(e) => {
                eprintln!("printf: {}", e);
                *has_error = true;
                (b"0".to_vec(), false, true)
            }
        },

        'u' => match parse_c_int(val) {
            Ok(n) => ((n as u64).to_string().into_bytes(), false, true),
            Err(e) => {
                eprintln!("printf: {}", e);
                *has_error = true;
                (b"0".to_vec(), false, true)
            }
        },

        'x' => match parse_c_int(val) {
            Ok(n) => (format!("{:x}", n as u64).into_bytes(), false, true),
            Err(e) => {
                eprintln!("printf: {}", e);
                *has_error = true;
                (b"0".to_vec(), false, true)
            }
        },

        'X' => match parse_c_int(val) {
            Ok(n) => (format!("{:X}", n as u64).into_bytes(), false, true),
            Err(e) => {
                eprintln!("printf: {}", e);
                *has_error = true;
                (b"0".to_vec(), false, true)
            }
        },

        'o' => match parse_c_int(val) {
            Ok(n) => (format!("{:o}", n as u64).into_bytes(), false, true),
            Err(e) => {
                eprintln!("printf: {}", e);
                *has_error = true;
                (b"0".to_vec(), false, true)
            }
        },

        'f' | 'F' => match val.trim().parse::<f64>() {
            Ok(n) => {
                // Default precision for f/F is 6 per POSIX/C standard.
                let s = if let Some(p) = prec {
                    format!("{:.*}", p, n)
                } else {
                    format!("{:.6}", n)
                };
                (s.into_bytes(), false, true)
            }
            Err(_) => {
                eprintln!("printf: '{}' expected numeric value", val);
                *has_error = true;
                (b"0.000000".to_vec(), false, true)
            }
        },

        'e' | 'E' => match val.trim().parse::<f64>() {
            Ok(n) => {
                // Scientific notation. Default precision is 6.
                // Rust's {:e} uses lowercase 'e', {:E} uses uppercase 'E'.
                let s = if let Some(p) = prec {
                    match spec {
                        'E' => format!("{:.*E}", p, n),
                        _ => format!("{:.*e}", p, n),
                    }
                } else {
                    match spec {
                        'E' => format!("{:.6E}", n),
                        _ => format!("{:.6e}", n),
                    }
                };
                (s.into_bytes(), false, true)
            }
            Err(_) => {
                eprintln!("printf: '{}' expected numeric value", val);
                *has_error = true;
                (b"0.000000e+00".to_vec(), false, true)
            }
        },

        'g' | 'G' => match val.trim().parse::<f64>() {
            Ok(n) => {
                // Default precision for g/G is 6 significant digits per POSIX.
                let p = prec.unwrap_or(6);
                // Note: Rust does not have a direct equivalent to C's %g that
                // automatically switches between %f and %e and strips trailing zeros.
                // Using {:.prec$} as a reasonable approximation for now.
                let s = format!("{:.*}", p, n);
                (s.into_bytes(), false, true)
            }
            Err(_) => {
                eprintln!("printf: '{}' expected numeric value", val);
                *has_error = true;
                (b"0.000000".to_vec(), false, true)
            }
        },

        'c' => {
            // If it contains one or more bytes, the first byte shall be written.
            let s = if let Some(&b) = val.as_bytes().first() {
                vec![b]
            } else {
                vec![]
            };
            (s, false, false)
        }

        'b' => {
            let (mut s, a) = interpret_escapes(val);
            if let Some(p) = prec {
                s.truncate(p);
            }
            (s, a, false)
        }

        'q' => (shell_quote(val), false, false),

        _ => {
            eprintln!("printf: unknown specifier %{}", spec);
            *has_error = true;
            (vec![], false, false)
        }
    };

    (pad_output(&s_bytes, width, zero, left, is_num), b_abort)
}

/// Expand C-style backslash escapes in a string (for the `%b` specifier).
/// Returns a tuple of the expanded byte string and a boolean indicating if
/// `\c` was encountered.
fn interpret_escapes(s: &str) -> (Vec<u8>, bool) {
    let mut result = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 1;
            match bytes[i] {
                b'a' => result.push(0x07),
                b'b' => result.push(0x08),
                b'f' => result.push(0x0c),
                b'n' => result.push(b'\n'),
                b'r' => result.push(b'\r'),
                b't' => result.push(b'\t'),
                b'v' => result.push(0x0b),
                b'\\' => result.push(b'\\'),
                b'c' => return (result, true),
                b'0' => {
                    // \0ddd: 0 to 3 octal digits
                    let mut octal = String::new();
                    let mut j = i + 1;
                    while j < bytes.len() && octal.len() < 3 && bytes[j] >= b'0' && bytes[j] <= b'7'
                    {
                        octal.push(bytes[j] as char);
                        j += 1;
                    }
                    if octal.is_empty() {
                        result.push(0);
                    } else {
                        let val = u8::from_str_radix(&octal, 8).unwrap_or(0);
                        result.push(val);
                    }
                    i = j - 1;
                }
                other => {
                    result.push(b'\\');
                    result.push(other);
                }
            }
        } else {
            result.push(bytes[i]);
        }
        i += 1;
    }

    (result, false)
}

/// Wrap a string in single quotes with interior quotes escaped for shell
/// consumption (the `%q` specifier).
fn shell_quote(s: &str) -> Vec<u8> {
    let mut result = Vec::with_capacity(s.len() + 8);
    result.push(b'\'');
    for &b in s.as_bytes() {
        if b == b'\'' {
            result.extend_from_slice(b"'\\''");
        } else {
            result.push(b);
        }
    }
    result.push(b'\'');
    result
}

register_command!(
    PRINTF_CMD,
    "printf",
    "^<1",
    CommandFlags::BIN.bits(),
    printf_main
);
