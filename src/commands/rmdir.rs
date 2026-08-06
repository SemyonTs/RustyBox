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
/// The option string requires at least one positional argument.
/// `(i)` provides a short alias for `--ignore-fail-on-non-empty`.
fn rmdir_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "<1(ignore-fail-on-non-empty)(i)p(parents)") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("rmdir: {e}");
            return 1;
        }
    };

    let flag_p = opts.count('p') > 0;
    // Check both the short alias 'i' and rely on the fact that our parser
    // maps the long option to 'i' via the (i) alias in optstr.
    let ignore_nonempty = opts.count('i') > 0;

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
/// Returns `true` on success. When `-p` is specified, stops ascending and
/// returns `true` if a parent cannot be removed because it is not empty or
/// due to permission errors (matching GNU rmdir behavior).
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

    // Track whether the original target was successfully removed.
    // For -p, we return success if the target was removed even if a parent
    // could not be (due to non-empty or permission issues).
    let mut target_removed = false;

    loop {
        match fs::remove_dir(current) {
            Ok(()) => {
                if !target_removed {
                    target_removed = true;
                }
            }
            Err(e) => {
                let kind = e.kind();

                // Suppress the error when the directory is non-empty and the
                // caller requested --ignore-fail-on-non-empty.
                if ignore_nonempty
                    && matches!(
                        kind,
                        std::io::ErrorKind::DirectoryNotEmpty | std::io::ErrorKind::Other
                    )
                {
                    return true;
                }

                // For -p: stop ascending silently on DirectoryNotEmpty or
                // PermissionDenied. Return success if the original target
                // was already removed.
                if parents
                    && matches!(
                        kind,
                        std::io::ErrorKind::DirectoryNotEmpty
                            | std::io::ErrorKind::PermissionDenied
                    )
                {
                    return target_removed;
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
    "<1(ignore-fail-on-non-empty)(i)p(parents)",
    CommandFlags::BIN.bits(),
    rmdir_main,
    description = "Remove empty directories",
    help = "\
OPTIONS:
-p                          Remove parent directories as well (similar to
                            `rmdir -p a/b/c` → removes `a/b/c`, `a/b`, `a`).
--ignore-fail-on-non-empty  Silently succeed when a directory is not empty.
"
);
