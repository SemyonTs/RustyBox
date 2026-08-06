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
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

/// Entry point for the `mv` builtin.
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

    let n = ctx.optargs.len();
    if n < 2 {
        eprintln!("mv: missing operand");
        return 1;
    }

    let sources = &ctx.optargs[..n - 1];
    let dest = &ctx.optargs[n - 1];

    let mut exit_code: u8 = 0;

    // Determine if destination is an existing directory.
    // fs::metadata follows symlinks, which matches POSIX behavior for target_dir.
    let dest_is_dir =
        !flag_T && (sources.len() > 1 || fs::metadata(dest).map(|m| m.is_dir()).unwrap_or(false));

    let mut target_buf = String::with_capacity(256);

    for src in sources {
        target_buf.clear();
        if dest_is_dir {
            target_buf.push_str(dest);
            target_buf.push('/');
            if let Some(base) = Path::new(src).file_name().and_then(|n| n.to_str()) {
                target_buf.push_str(base);
            } else {
                target_buf.push_str(src);
            }
        } else {
            target_buf.push_str(dest);
        }

        if let Err(e) = move_one(src, &target_buf, flag_f, flag_i, flag_n, flag_v) {
            eprintln!("mv: {e}");
            exit_code = 1;
        }
    }

    exit_code
}

/// Move a single filesystem entry from `src` to `dest`.
fn move_one(
    src: &str,
    dest: &str,
    flag_f: bool,
    flag_i: bool,
    flag_n: bool,
    flag_v: bool,
) -> Result<(), String> {
    let src_path = Path::new(src);
    let dest_path = Path::new(dest);

    let src_is_dir = src_path.is_dir();
    let dest_exists = dest_path.exists() || dest_path.symlink_metadata().is_ok();

    // POSIX: if source is a non-directory and target ends with a slash, it's an error.
    if !src_is_dir && dest.ends_with('/') {
        return Err(format!("cannot move '{}' to a directory", src));
    }

    // POSIX: if dest is a directory and source is not, it's an error
    // (Note: mv_main already handles appending basename if dest is a dir,
    // so this catches the case where the constructed dest path happens to be an existing dir).
    if dest_exists && dest_path.is_dir() && !src_is_dir {
        return Err(format!(
            "cannot overwrite directory '{}' with non-directory '{}'",
            dest, src
        ));
    }

    if dest_exists {
        if flag_n {
            return Ok(()); // No-clobber
        }

        if flag_i {
            eprint!("mv: overwrite '{}'? ", dest);
            let mut buf = String::new();
            std::io::stdin().read_line(&mut buf).ok();
            if !buf.trim().to_lowercase().starts_with('y') {
                return Ok(());
            }
        }

        if flag_f {
            // Safely remove only the specific file or symlink.
            // Do NOT use remove_dir_all here, as it would destroy directory contents.
            // If it's a non-empty directory, rename() will fail later, which is correct POSIX behavior.
            let _ = fs::remove_file(dest);
        }
    }

    if flag_v {
        eprintln!("renamed '{}' -> '{}'", src, dest);
    }

    // Fast path: atomic rename (same filesystem)
    match fs::rename(src, dest) {
        Ok(()) => Ok(()),
        Err(_) => {
            // Cross-filesystem fallback: duplicate hierarchy, then remove source.
            copy_hierarchy(src_path, dest_path)?;

            // Remove original source
            if src_is_dir {
                fs::remove_dir_all(src)
                    .map_err(|e| format!("failed to remove source dir '{}': {}", src, e))?;
            } else {
                fs::remove_file(src)
                    .map_err(|e| format!("failed to remove source '{}': {}", src, e))?;
            }
            Ok(())
        }
    }
}

/// Recursively copy a file hierarchy, preserving symlinks as symlinks (POSIX requirement).
fn copy_hierarchy(src: &Path, dest: &Path) -> Result<(), String> {
    let meta =
        fs::symlink_metadata(src).map_err(|e| format!("cannot stat '{}': {}", src.display(), e))?;

    if meta.file_type().is_symlink() {
        let target = fs::read_link(src)
            .map_err(|e| format!("cannot read symlink '{}': {}", src.display(), e))?;
        symlink(target, dest)
            .map_err(|e| format!("cannot create symlink '{}': {}", dest.display(), e))?;
    } else if meta.is_dir() {
        fs::create_dir_all(dest)
            .map_err(|e| format!("cannot create directory '{}': {}", dest.display(), e))?;
        for entry in
            fs::read_dir(src).map_err(|e| format!("cannot read dir '{}': {}", src.display(), e))?
        {
            let entry = entry.map_err(|e| format!("cannot read dir entry: {}", e))?;
            copy_hierarchy(&entry.path(), &dest.join(entry.file_name()))?;
        }
    } else {
        fs::copy(src, dest).map_err(|e| format!("cannot copy file '{}': {}", src.display(), e))?;
    }

    // Optional: preserve basic metadata (timestamps, permissions)
    if let Ok(meta) = fs::metadata(src) {
        let _ = fs::set_permissions(dest, meta.permissions());
    }

    Ok(())
}

register_command!(
    MV_CMD,
    "mv",
    "finTv",
    CommandFlags::BIN.bits(),
    mv_main,
    description = "Move (rename) files",
    help = "\
OPTIONS:
-f      Force overwrite of existing destination files.
-i      Prompt before overwriting.
-n      Do not overwrite an existing file (no-clobber).
-v      Verbose: print each source/destination pair.
-T      Treat the destination as a normal file, not a directory."
);
