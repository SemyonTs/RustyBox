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

    // Logical path: prefer $PWD when it is a valid alias for the physical
    // directory and -P was not requested.
    if !flag_p {
        if let Ok(pwd_env) = env::var("PWD") {
            if is_valid_pwd(&cwd, &pwd_env) {
                println!("{pwd_env}");
                return 0;
            }
        }
    }

    // Fall back to physical path.
    println!("{}", cwd.display());
    0
}

/// Verify that `pwd_env` is a legitimate logical path for `cwd`.
///
/// The value must be an absolute path that does not contain `.` or `..`
/// components, and it must resolve to the same device/inode pair as `cwd`.
fn is_valid_pwd(cwd: &std::path::Path, pwd_env: &str) -> bool {
    // Must be absolute.
    if !pwd_env.starts_with('/') {
        return false;
    }

    // Reject any path containing a literal `.` or `..` segment.
    for seg in pwd_env.split('/').skip(1) {
        if seg == "." || seg == ".." {
            return false;
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
