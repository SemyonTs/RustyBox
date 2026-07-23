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
    let mut path = name.to_string();

    loop {
        match fs::remove_dir(&path) {
            Ok(()) => {}
            Err(e) => {
                // Suppress the error when the directory is non-empty and the
                // caller requested `--ignore-fail-on-non-empty`.
                let is_nonempty = matches!(
                    e.kind(),
                    std::io::ErrorKind::DirectoryNotEmpty | std::io::ErrorKind::Other
                ) && (e.raw_os_error() == Some(39) // ENOTEMPTY on Linux
                    || e.to_string().contains("not empty"));

                if ignore_nonempty && is_nonempty {
                    return true;
                }

                eprintln!("rmdir: cannot remove '{}': {}", path, e);
                return false;
            }
        }

        if !parents {
            return true;
        }

        // Ascend one level: strip the last path component.
        // Trailing slashes are removed first so they do not interfere.
        while path.ends_with('/') {
            path.pop();
        }

        match path.rfind('/') {
            Some(i) => {
                if i == 0 {
                    // The remainder is "/" or an immediate child of root
                    // (e.g. "/a" → "/").
                    if path == "/" {
                        return true;
                    }
                    path.truncate(i);
                    if path.is_empty() {
                        return true;
                    }
                } else {
                    path.truncate(i);
                }
            }
            None => return true, // no more ancestors to process
        }

        // Stop when we reach the filesystem root.
        if path.is_empty() || path == "/" {
            return true;
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
