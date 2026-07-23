// =============================================================================
// pwd — Print the current working directory.
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
//   -L   Print the logical path (from $PWD, if valid).  This is the default.
//   -P   Print the physical path (no symlink resolution).
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::env;
use std::fs;
use std::os::unix::fs::MetadataExt;

/// Entry point for the `pwd` builtin.
///
/// The option string `">0LP"` forbids positional arguments.  By default the
/// logical path from `$PWD` is used; `-P` forces the physical path via
/// `getcwd(3)`.
fn pwd_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, ">0LP") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("pwd: {e}");
            return 1;
        }
    };

    let flag_p = opts.count('P') > 0;

    // Physical path (always available).
    let cwd = match env::current_dir() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("pwd: {e}");
            return 1;
        }
    };
    let cwd_str = cwd.to_string_lossy().into_owned();

    // Logical path: prefer $PWD when it is a valid alias for the physical
    // directory and -P was not requested.
    let logical = if !flag_p {
        env::var("PWD")
            .ok()
            .filter(|pwd_env| is_valid_pwd(&cwd_str, pwd_env))
    } else {
        None
    };

    let result = logical.unwrap_or(cwd_str);
    println!("{result}");

    0
}

/// Verify that `pwd_env` is a legitimate logical path for `cwd`.
///
/// The value must be an absolute path that does not contain `.` or `..`
/// components, and it must resolve to the same device/inode pair as `cwd`.
fn is_valid_pwd(cwd: &str, pwd_env: &str) -> bool {
    // Must be absolute.
    if !pwd_env.starts_with('/') {
        return false;
    }

    // Reject any path containing a literal `.` or `..` segment.
    let mut s = pwd_env;
    while let Some(rest) = s.strip_prefix('/') {
        if let Some(tail) = rest.strip_prefix('.') {
            if tail.is_empty() || tail.starts_with('/') {
                return false;
            }
            if let Some(t2) = tail.strip_prefix('.') {
                if t2.is_empty() || t2.starts_with('/') {
                    return false;
                }
            }
        }
        // Advance to the next slash-delimited segment.
        match rest.find('/') {
            Some(i) => s = &rest[i..],
            None => break,
        }
    }

    // Compare the device and inode of both paths.
    let st1 = match fs::metadata(cwd) {
        Ok(m) => m,
        Err(_) => return false,
    };
    let st2 = match fs::metadata(pwd_env) {
        Ok(m) => m,
        Err(_) => return false,
    };

    st1.dev() == st2.dev() && st1.ino() == st2.ino()
}

register_command!(
    PWD_CMD,
    "pwd",
    ">0LP",
    CommandFlags::BIN.bits() | CommandFlags::MAYFORK.bits(),
    pwd_main
);
