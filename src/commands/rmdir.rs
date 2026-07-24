// =============================================================================
// rmdir — Remove empty directories.
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
//   -p                          Remove parent directories as well (similar to
//                               `rmdir -p a/b/c` → removes `a/b/c`, `a/b`, `a`).
//   --ignore-fail-on-non-empty  Silently succeed when a directory is not empty.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::fs;

/// Entry point for the `rmdir` builtin.
///
/// The option string `"<1(ignore-fail-on-non-empty)p(parents)"` requires at
/// least one positional argument.
fn rmdir_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "<1(ignore-fail-on-non-empty)p(parents)") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("rmdir: {e}");
            return 1;
        }
    };

    let flag_p = opts.count('p') > 0;
    let ignore_nonempty = opts.count('i') > 0; // longopt ignore-fail-on-non-empty

    let mut exit_code: u8 = 0;
    for name in &ctx.optargs {
        if !do_rmdir(name, flag_p, ignore_nonempty) {
            exit_code = 1;
        }
    }

    exit_code
}

/// Remove a directory, and optionally its ancestors when `parents` is true.
///
/// Returns `true` on success.
fn do_rmdir(name: &str, parents: bool, ignore_nonempty: bool) -> bool {
    // Work with a &str slice into the original path, only allocating when
    // the path contains trailing slashes that need trimming.
    let base_path: String;
    let mut current: &str = if name.ends_with('/') {
        base_path = name.trim_end_matches('/').to_string();
        if base_path.is_empty() {
            return true; // name was all slashes — nothing to do.
        }
        &base_path
    } else {
        name
    };

    loop {
        match fs::remove_dir(current) {
            Ok(()) => {}
            Err(e) => {
                // Suppress the error when the directory is non-empty and the
                // caller requested `--ignore-fail-on-non-empty`.
                if ignore_nonempty
                    && matches!(
                        e.kind(),
                        std::io::ErrorKind::DirectoryNotEmpty | std::io::ErrorKind::Other
                    )
                {
                    return true;
                }

                eprintln!("rmdir: cannot remove '{}': {}", current, e);
                return false;
            }
        }

        if !parents {
            return true;
        }

        // Ascend one level: strip the last path component.
        match current.rsplit_once('/') {
            Some((parent, _)) => {
                if parent.is_empty() {
                    // current was "/foo" or just "/" — root reached.
                    return true;
                }
                current = parent;
            }
            None => return true, // no more ancestors
        }
    }
}

register_command!(
    RMDIR_CMD,
    "rmdir",
    "<1(ignore-fail-on-non-empty)p(parents)",
    CommandFlags::BIN.bits(),
    rmdir_main
);
