// =============================================================================
// dirname — Return the directory portion of a pathname.
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Implementation inspired by Toybox (https://landley.net/toybox/)
// Toybox is copyrighted by Rob Landley, see NOTICE file for license details.
//
// Full POSIX semantics for all edge cases.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;

/// Entry point for the `dirname` builtin.
///
/// The option string `"<1"` enforces at least one positional argument.
fn dirname_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "<1") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("dirname: {e}");
            return 1;
        }
    };

    // Collect output lines into a single buffer to minimize write syscalls.
    // dirname output is tiny (a few bytes per line), so buffering here is
    // mostly about syscall reduction, not throughput.
    let mut out = String::with_capacity(ctx.optargs.len() * 32);
    for path in &ctx.optargs {
        out.push_str(posix_dirname(path));
        out.push('\n');
    }
    print!("{out}");

    let _ = opts;
    0
}

/// Return the directory portion of `path` according to POSIX semantics.
///
/// Edge-case behaviour:
///   - `""`        → `"."`
///   - `"/"`       → `"/"`
///   - `"//"`      → `"/"`
///   - `"/usr/"`   → `"/"`
///   - `"/usr/lib"`→ `"/usr"`
///   - `"usr/lib"` → `"usr"`
///   - `"usr"`     → `"."`
///   - `"a/b/"`    → `"a"`
fn posix_dirname(path: &str) -> &str {
    if path.is_empty() {
        return ".";
    }

    // Strip trailing slashes so they do not interfere with the split.
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        // The original string consisted entirely of slashes.
        return "/";
    }

    // A path with no slashes after trimming has no directory component.
    // This also covers the case where the only slash was trailing, e.g. "usr/".
    if !trimmed.contains('/') {
        return ".";
    }

    // Find the last slash and return everything before it.
    match trimmed.rsplit_once('/') {
        Some((dir, _)) => {
            if dir.is_empty() {
                "/"
            } else {
                dir
            }
        }
        None => "/",
    }
}

register_command!(
    DIRNAME_CMD,
    "dirname",
    "<1",
    CommandFlags::BIN.bits() | CommandFlags::USR.bits(),
    dirname_main
);
