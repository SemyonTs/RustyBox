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

use std::io::Write;

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

    let stdout = std::io::stdout();
    let mut writer = std::io::BufWriter::new(stdout.lock());
    let mut ret: u8 = 0;

    // Write all arguments separated by a single space.
    for (i, arg) in ctx.optargs.iter().enumerate() {
        if i > 0 {
            if writer.write_all(b" ").is_err() {
                ret = 1;
                break;
            }
        }
        if interpret {
            if write_interpreted(&mut writer, arg).is_err() {
                ret = 1;
                break;
            }
        } else {
            if writer.write_all(arg.as_bytes()).is_err() {
                ret = 1;
                break;
            }
        }
    }

    if ret == 0 {
        if !no_newline {
            if writer.write_all(b"\n").is_err() {
                ret = 1;
            }
        }
        if writer.flush().is_err() {
            ret = 1;
        }
    } else {
        // If we already failed, we still flush to avoid leaving the buffer
        // in a broken state, but we ignore the result.
        let _ = writer.flush();
    }

    ret
}

/// Write `s` to `writer`, expanding a limited set of C-style backslash escapes.
///
/// Recognised sequences:
///   `\n`  — newline
///   `\t`  — horizontal tab
///   `\r`  — carriage return
///   `\\`  — literal backslash
///
/// An unrecognised escape character is reproduced literally (the backslash
/// is preserved).
fn write_interpreted(writer: &mut impl std::io::Write, s: &str) -> std::io::Result<()> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 1;
            let b = match bytes[i] {
                b'n' => b'\n',
                b't' => b'\t',
                b'r' => b'\r',
                b'\\' => b'\\',
                _ => {
                    writer.write_all(&[b'\\', bytes[i]])?;
                    i += 1;
                    continue;
                }
            };
            writer.write_all(&[b])?;
        } else {
            writer.write_all(&[bytes[i]])?;
        }
        i += 1;
    }
    Ok(())
}

register_command!(
    ECHO_CMD,
    "echo",
    "^?ne",
    CommandFlags::BIN.bits() | CommandFlags::MAYFORK.bits() | CommandFlags::LINEBUF.bits(),
    echo_main,
    description = "Display a line of text",
    help = "\
OPTIONS:
-n   Do not output the trailing newline.
-e   Enable interpretation of backslash-escaped characters
      (\n, \t, \r, \\).  Without -e the input is printed literally."
);
