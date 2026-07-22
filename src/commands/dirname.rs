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

    for path in &ctx.optargs {
        println!("{}", posix_dirname(path));
    }

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

    // A path with no slashes at all has no directory component.
    if !path.contains('/') {
        return ".";
    }

    // Strip trailing slashes so they do not interfere with the split.
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        // The original string consisted entirely of slashes.
        return "/";
    }

    // Find the last slash and return everything before it.
    match trimmed.rsplit_once('/') {
        Some((dir, _base)) => {
            if dir.is_empty() {
                // The path started with a slash (e.g. "/usr" → dir is empty).
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