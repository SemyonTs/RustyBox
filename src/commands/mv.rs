// =============================================================================
// mv — Move (rename) files.
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
//   -f      Force overwrite of existing destination files.
//   -i      Prompt before overwriting.
//   -n      Do not overwrite an existing file (no-clobber).
//   -v      Verbose: print each source/destination pair.
//   -T      Treat the destination as a normal file, not a directory.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::fs;
use std::path::{Path, PathBuf};

/// Entry point for the `mv` builtin.
///
/// Attempts an atomic `rename(2)` first; falls back to copy-and-delete for
/// cross-filesystem moves.
fn mv_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "finTv") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("mv: {e}");
            return 1;
        }
    };

    let flag_f = opts.count('f') > 0;
    let flag_i = opts.count('i') > 0;
    let flag_n = opts.count('n') > 0;
    let flag_v = opts.count('v') > 0;
    let flag_T = opts.count('T') > 0;

    let mut args: Vec<String> = ctx.optargs.clone();
    if args.len() < 2 {
        eprintln!("mv: not enough arguments");
        return 1;
    }

    let dest = args.pop().unwrap();
    let sources = args;

    let mut exit_code: u8 = 0;

    // Determine whether the destination is an existing directory (unless -T
    // forces file semantics).
    let dest_is_dir =
        !flag_T && (sources.len() > 1 || fs::metadata(&dest).map(|m| m.is_dir()).unwrap_or(false));

    for src in &sources {
        let target = if dest_is_dir {
            Path::new(&dest).join(Path::new(src).file_name().unwrap_or_default())
        } else {
            PathBuf::from(&dest)
        };

        if let Err(e) = move_one(
            src,
            &target.to_string_lossy(),
            flag_f,
            flag_i,
            flag_n,
            flag_v,
        ) {
            eprintln!("mv: {e}");
            exit_code = 1;
        }
    }

    exit_code
}

/// Move a single filesystem entry from `src` to `dest`.
///
/// If the destination exists and is a directory while the source is not, the
/// operation is redirected into that directory.  The move is first attempted
/// via `rename(2)`; on cross-device errors a byte-for-byte copy followed by
/// removal of the source is used as a fallback.
fn move_one(
    src: &str,
    dest: &str,
    flag_f: bool,
    flag_i: bool,
    flag_n: bool,
    flag_v: bool,
) -> Result<(), String> {
    // Resolve destination-directory redirection.
    if let Ok(dmeta) = fs::symlink_metadata(dest) {
        if dmeta.is_dir() && !Path::new(src).is_dir() {
            let new_dest = Path::new(dest).join(Path::new(src).file_name().unwrap_or_default());
            return move_one(
                src,
                &new_dest.to_string_lossy(),
                flag_f,
                flag_i,
                flag_n,
                flag_v,
            );
        }

        // -n: no-clobber.
        if flag_n {
            return Ok(());
        }

        // -i: interactive prompt.
        if flag_i {
            eprint!("mv: overwrite '{}'? ", dest);
            let mut buf = String::new();

            std::io::stdin().read_line(&mut buf).ok();
            if !buf.trim().starts_with('y') {
                return Ok(());
            }
        }

        // -f: forcibly remove the existing destination.
        if flag_f {
            let _ = fs::remove_file(dest);
            let _ = fs::remove_dir_all(dest);
        }
    }

    if flag_v {
        eprintln!("renamed '{}' -> '{}'", src, dest);
    }

    // Fast path: atomic rename (works when both paths reside on the same
    // filesystem).
    match fs::rename(src, dest) {
        Ok(()) => Ok(()),
        Err(_) => {
            // Cross-filesystem fallback: copy then delete the source.
            fs::copy(src, dest).map_err(|e| format!("'{}': {}", dest, e))?;
            fs::remove_file(src).map_err(|e| format!("'{}': {}", src, e))?;
            Ok(())
        }
    }
}

register_command!(MV_CMD, "mv", "finTv", CommandFlags::BIN.bits(), mv_main);
