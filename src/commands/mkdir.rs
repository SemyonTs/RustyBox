// =============================================================================
// mkdir — Create directories.
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
//   -p        Create parent directories as needed (no error if existing).
//   -m MODE   Set the file permission bits of the new directory to MODE
//             (octal or symbolic, e.g. `0755` or `u=rwx,go=rx`).
//   -v        Print a message for each created directory.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::fs;
use std::os::unix::fs::DirBuilderExt;

/// Entry point for the `mkdir` builtin.
///
/// The option string `"<1vp(parent)(parents)m:"` requires at least one
/// positional argument.  `(parent)` and `(parents)` are long-option aliases
/// for `-p`.
fn mkdir_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "<1vp(parent)(parents)m:") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("mkdir: {e}");
            return 1;
        }
    };

    let flag_p = opts.count('p') > 0;
    let flag_v = opts.count('v') > 0;
    let mode_arg = opts.get_str('m').unwrap_or("");

    // Determine the default mode: 0777 masked by the current umask.
    let umask = unsafe { libc::umask(0) };
    unsafe {
        libc::umask(umask);
    }
    let default_mode: u32 = 0o777 & !umask as u32;

    let mode: u32 = if mode_arg.is_empty() {
        default_mode
    } else {
        match parse_mode(mode_arg, default_mode) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("mkdir: invalid mode '{}': {}", mode_arg, e);
                return 1;
            }
        }
    };

    let mut exit_code: u8 = 0;
    for name in &ctx.optargs {
        if !make_dir(name, flag_p, mode, flag_v) {
            exit_code = 1;
        }
    }

    exit_code
}

/// Create a single directory (and its ancestors when `parents` is true).
///
/// Returns `true` on success.
fn make_dir(path: &str, parents: bool, mode: u32, verbose: bool) -> bool {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(parents);
    builder.mode(mode);

    if let Err(e) = builder.create(path) {
        eprintln!("mkdir: cannot create directory '{}': {}", path, e);
        return false;
    }

    if verbose {
        println!("mkdir: created directory '{}'", path);
    }

    true
}

/// Parse a mode argument that is either an octal string or a symbolic
/// clause list (e.g. `u+rwx,g-w,o=rx`).
///
/// The `base` parameter supplies the starting mode used as the reference
/// for symbolic operations.
fn parse_mode(s: &str, base: u32) -> Result<u32, String> {
    // Octal: all characters are ASCII digits (POSIX treats these as octal).
    if s.chars().all(|c| c.is_ascii_digit()) {
        return u32::from_str_radix(s, 8).map_err(|_| "invalid octal".to_string());
    }

    // Leading-zero octal form.
    if let Some(rest) = s.strip_prefix("0") {
        if rest.chars().all(|c| c.is_ascii_digit()) {
            return u32::from_str_radix(rest, 8).map_err(|_| "invalid octal".to_string());
        }
    }

    // Otherwise treat as symbolic mode: comma-separated [ugoa]*[+-=][rwx]*.
    parse_symbolic(s, base)
}

/// Parse a symbolic mode string such as `u+rwx,g-w,o=rx` or `a=rwx`.
///
/// Each clause is applied in order to the supplied `base` mode.
fn parse_symbolic(s: &str, base: u32) -> Result<u32, String> {
    let mut mode = base;

    for clause in s.split(',') {
        let clause = clause.trim();
        if clause.is_empty() {
            continue;
        }

        // Locate the boundary between the who-list and the operator.
        let mut who_end = 0;
        let bytes = clause.as_bytes();
        while who_end < bytes.len() {
            match bytes[who_end] {
                b'u' | b'g' | b'o' | b'a' => who_end += 1,
                _ => break,
            }
        }

        let who = &clause[..who_end];
        let rest = &clause[who_end..];

        if rest.is_empty() {
            return Err(format!("invalid mode clause '{}'", clause));
        }

        let op = rest.as_bytes()[0];
        if !matches!(op, b'+' | b'-' | b'=') {
            return Err(format!("expected +, -, or = in '{}'", clause));
        }

        let perms = &rest[1..];

        // Build the bitmask for the affected classes.
        let mut mask: u32 = 0;
        if who.is_empty() || who.contains('a') {
            mask = 0o777;
        } else {
            if who.contains('u') {
                mask |= 0o700;
            }
            if who.contains('g') {
                mask |= 0o070;
            }
            if who.contains('o') {
                mask |= 0o007;
            }
        }

        // Collect the requested permission bits.
        let mut bits: u32 = 0;
        for c in perms.chars() {
            match c {
                'r' => bits |= 0o444,
                'w' => bits |= 0o222,
                'x' => bits |= 0o111,
                _ => return Err(format!("unknown permission '{}'", c)),
            }
        }

        // Restrict to only the bits relevant for the chosen who-classes.
        bits &= mask;

        match op {
            b'+' => mode |= bits,
            b'-' => mode &= !bits,
            b'=' => {
                mode &= !mask;
                mode |= bits;
            }
            _ => unreachable!(),
        }
    }

    Ok(mode)
}

register_command!(
    MKDIR_CMD,
    "mkdir",
    "<1vp(parent)(parents)m:",
    CommandFlags::BIN.bits(),
    mkdir_main
);