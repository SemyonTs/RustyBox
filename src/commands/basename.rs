// =============================================================================
// basename — Return the non-directory portion of a pathname.
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Implementation inspired by Toybox (https://landley.net/toybox/)
// Toybox is copyrighted by Rob Landley, see NOTICE file for license details.
//
// Full POSIX behavior:
//   -a       Treat every argument as a pathname (multiple-output mode).
//   -s SUF   Strip SUF from the end of each resulting filename.
//            Implies -a.
//   Default  Two-argument form: basename NAME [SUFFIX].
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;

/// Entry point for the `basename` builtin.
///
/// Parses the option string `^<1as:` which enforces:
///   - `^<1`   : at least one positional argument,
///   - `a`     : multi-operand flag (bit 1),
///   - `s:`    : suffix removal, expects a string argument (bit 0).
fn basename_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "^<1as:") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("basename: {e}");
            return 1;
        }
    };

    let flag_a = opts.count('a') > 0;
    let suffix = opts.get_str('s').unwrap_or("");

    // The -s option implies multi-operand mode (-a).
    let all = flag_a || !suffix.is_empty();

    let args: Vec<String> = if all {
        ctx.optargs.clone()
    } else {
        // Two-argument form: basename NAME [SUFFIX].
        if ctx.optargs.len() > 2 {
            eprintln!("basename: too many args");
            return 1;
        }
        let suf = ctx.optargs.get(1).map(|s| s.as_str()).unwrap_or("");
        let name = ctx.optargs[0].clone();
        let base = posix_basename(&name);
        let trimmed = strip_suffix(base, suf);
        println!("{trimmed}");
        return 0;
    };

    for name in &args {
        let base = posix_basename(name);
        let trimmed = strip_suffix(base, suffix);
        println!("{trimmed}");
    }

    0
}

/// Return the final component of `path`, with trailing slashes removed.
///
/// Implements POSIX semantics:
///   - `""`       → `""`
///   - `"/"`      → `"/"`
///   - `"///"`    → `"/"`
///   - `"/usr/"`  → `"usr"`
///   - `"usr"`    → `"usr"`
fn posix_basename(path: &str) -> &str {
    if path.is_empty() {
        return "";
    }

    // Strip trailing slashes.
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        // The entire input consisted of slashes.
        return "/";
    }

    match trimmed.rsplit_once('/') {
        Some((_, base)) => base,
        None => trimmed,
    }
}

/// Strip `suffix` from `base` if:
///   - `suffix` is non-empty,
///   - `base` ends with `suffix`,
///   - the remaining prefix is non-empty.
///
/// Otherwise return `base` unchanged.
fn strip_suffix<'a>(base: &'a str, suffix: &str) -> &'a str {
    if !suffix.is_empty() {
        if let Some(stripped) = base.strip_suffix(suffix) {
            if !stripped.is_empty() {
                return stripped;
            }
        }
    }
    base
}

register_command!(
    BASENAME_CMD,
    "basename",
    "^<1as:",
    CommandFlags::BIN.bits() | CommandFlags::USR.bits(),
    basename_main
);
