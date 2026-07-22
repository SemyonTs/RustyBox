// =============================================================================
// echo — Display a line of text.
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
//   -n   Do not output the trailing newline.
//   -e   Enable interpretation of backslash-escaped characters
//        (\n, \t, \r, \\).  Without -e the input is printed literally.
// =============================================================================

use crate::args::ParsedOpts;
use crate::context::Context;
use crate::flags::CommandFlags;

/// Entry point for the `echo` builtin.
///
/// Option string `"^?ne"`:
///   - `^`   Stop option processing at the first non-option argument.
///   - `n`   Suppress trailing newline (bit 1).
///   - `e`   Enable escape-sequence interpretation (bit 0).
fn echo_main(ctx: &mut Context) -> u8 {
    let opts: ParsedOpts = match crate::args::parse(ctx, "^?ne") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("echo: {e}");
            return 1;
        }
    };

    let no_newline = opts.count('n') > 0;
    let interpret = opts.count('e') > 0;

    // Build the output string by joining all arguments with a single space.
    let mut out = String::new();
    for (i, arg) in ctx.optargs.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        if interpret {
            push_interpreted(&mut out, arg);
        } else {
            out.push_str(arg);
        }
    }

    if no_newline {
        print!("{out}");
    } else {
        println!("{out}");
    }

    0
}

/// Append `s` to `out`, expanding a limited set of C-style backslash escapes.
///
/// Recognised sequences:
///   `\n`  — newline
///   `\t`  — horizontal tab
///   `\r`  — carriage return
///   `\\`  — literal backslash
///
/// An unrecognised escape character is reproduced literally (the backslash
/// is preserved).
fn push_interpreted(out: &mut String, s: &str) {
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
}

register_command!(
    ECHO_CMD,
    "echo",
    "^?ne",
    CommandFlags::BIN.bits() | CommandFlags::MAYFORK.bits() | CommandFlags::LINEBUF.bits(),
    echo_main
);