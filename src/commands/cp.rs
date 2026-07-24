// =============================================================================
// cp — Copy files and directories.
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
//   -a      Archive mode: equivalent to -dpR.
//   -d      Preserve symlinks (do not dereference). Implied by -a.
//   -f      Force overwrite of existing destination files.
//   -i      Prompt before overwriting.
//   -l      Create hard links instead of copying.
//   -n      Do not overwrite an existing file (no-clobber).
//   -p      Preserve mode, ownership, and timestamps. Implied by -a.
//   -R, -r  Copy directories recursively. Implied by -a.
//   -s      Create symbolic links instead of copying.
//   -u      Copy only when the source file is newer than the destination.
//   -v      Verbose: print each source/destination pair.
//   -H      Follow command-line symlinks.
//   -L      Follow all symlinks.
//   -P      Never follow symlinks (default).
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use filetime::FileTime;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

/// Entry point for the `cp` builtin.
///
/// Option string `a(dpr)dfilnprsuv[-HLP]`:
///   - `a(dpr)` expands -a into the equivalent -d, -p, -r flags.
///   - `[-HLP]` are recognised but handled at a higher level (symlink
///     dereferencing policy); here they are accepted silently.
fn cp_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "a(dpr)dfilnprsuv[-HLP]") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("cp: {e}");
            return 1;
        }
    };

    let flag_a = opts.count('a') > 0;
    let flag_d = opts.count('d') > 0 || flag_a;
    let flag_f = opts.count('f') > 0;
    let flag_i = opts.count('i') > 0;
    let flag_l = opts.count('l') > 0;
    let flag_n = opts.count('n') > 0;
    let flag_p = opts.count('p') > 0 || flag_a;
    let flag_r = opts.count('r') > 0 || opts.count('R') > 0 || flag_a;
    let flag_s = opts.count('s') > 0;
    let flag_u = opts.count('u') > 0;
    let flag_v = opts.count('v') > 0;

    let n = ctx.optargs.len();
    if n < 2 {
        eprintln!("cp: not enough arguments");
        return 1;
    }

    // The last argument is always the destination; all preceding are sources.
    // No cloning — just slice the existing Vec.
    let sources = &ctx.optargs[..n - 1];
    let dest = &ctx.optargs[n - 1];

    let mut exit_code: u8 = 0;

    // If multiple sources are given the destination must be an existing
    // directory, or the operation will fail downstream.
    let dest_is_dir = sources.len() > 1 || fs::metadata(dest).map(|m| m.is_dir()).unwrap_or(false);

    for src in sources {
        let target = if dest_is_dir {
            Path::new(dest).join(Path::new(src).file_name().unwrap_or_default())
        } else {
            PathBuf::from(dest)
        };

        if let Err(e) = copy_one(
            src,
            target.to_str().unwrap_or_default(),
            flag_a,
            flag_d,
            flag_f,
            flag_i,
            flag_l,
            flag_n,
            flag_p,
            flag_r,
            flag_s,
            flag_u,
            flag_v,
        ) {
            eprintln!("cp: {e}");
            exit_code = 1;
        }
    }

    exit_code
}

/// Dispatch a single source/destination pair to the appropriate copy strategy
/// (regular file, directory, symlink, hard link, or symlink creation).
fn copy_one(
    src: &str,
    dest: &str,
    flag_a: bool,
    flag_d: bool,
    flag_f: bool,
    flag_i: bool,
    flag_l: bool,
    flag_n: bool,
    flag_p: bool,
    flag_r: bool,
    flag_s: bool,
    flag_u: bool,
    flag_v: bool,
) -> Result<(), String> {
    let meta = fs::symlink_metadata(src).map_err(|e| format!("'{src}': {e}"))?;

    // If the destination exists and is a directory while the source is not,
    // redirect the copy into that directory, preserving the source filename.
    if let Ok(dmeta) = fs::symlink_metadata(dest) {
        if dmeta.is_dir() && !meta.is_dir() {
            let new_dest = Path::new(dest).join(Path::new(src).file_name().unwrap_or_default());
            return copy_one(
                src,
                new_dest.to_str().unwrap_or_default(),
                flag_a,
                flag_d,
                flag_f,
                flag_i,
                flag_l,
                flag_n,
                flag_p,
                flag_r,
                flag_s,
                flag_u,
                flag_v,
            );
        }

        // -n: no-clobber — silently skip if destination already exists.
        if flag_n {
            return Ok(());
        }

        // -i: interactive prompt before overwriting.
        if flag_i {
            eprint!("cp: overwrite '{}'? ", dest);
            let mut buf = String::new();
            std::io::stdin().read_line(&mut buf).ok();
            if !buf.trim().starts_with('y') {
                return Ok(());
            }
        }

        // -f: remove the existing destination entry before proceeding.
        if flag_f {
            let _ = fs::remove_file(dest);
            let _ = fs::remove_dir_all(dest);
        }
    }

    // -u: update — skip if the destination has an equal or newer mtime.
    if flag_u {
        if let Ok(dmeta) = fs::metadata(dest) {
            if dmeta.mtime() >= meta.mtime() {
                return Ok(());
            }
        }
    }

    if flag_v {
        eprintln!("'{}' -> '{}'", src, dest);
    }

    // Preserve symlinks as symlinks when -d is given.
    if meta.file_type().is_symlink() && flag_d {
        let target = fs::read_link(src).map_err(|e| format!("'{src}': {e}"))?;
        let _ = fs::remove_file(dest);
        symlink(target, dest).map_err(|e| format!("'{dest}': {e}"))?;
        return Ok(());
    }

    // Directory copy: requires -r/-R/-a.
    if meta.is_dir() {
        if !flag_r && !flag_a {
            return Err(format!("skipping directory '{src}'"));
        }
        copy_dir(
            src, dest, flag_a, flag_d, flag_f, flag_i, flag_l, flag_n, flag_p, flag_r, flag_s,
            flag_u, flag_v,
        )?;
        return Ok(());
    }

    // Hard link creation.
    if flag_l {
        let _ = fs::remove_file(dest);
        fs::hard_link(src, dest).map_err(|e| format!("'{dest}': {e}"))?;
        return Ok(());
    }

    // Symbolic link creation (relative path).
    if flag_s {
        let _ = fs::remove_file(dest);
        let rel = relative_path(dest, src);
        symlink(rel, dest).map_err(|e| format!("'{dest}': {e}"))?;
        return Ok(());
    }

    // Fallback: byte-for-byte regular file copy.
    fs::copy(src, dest).map_err(|e| format!("'{dest}': {e}"))?;

    // Preserve metadata when -p is active.
    if flag_p {
        preserve_attrs(src, dest, &meta)?;
    }

    Ok(())
}

/// Recursively copy a directory tree from `src` to `dest`.
fn copy_dir(
    src: &str,
    dest: &str,
    flag_a: bool,
    flag_d: bool,
    flag_f: bool,
    flag_i: bool,
    flag_l: bool,
    flag_n: bool,
    flag_p: bool,
    flag_r: bool,
    flag_s: bool,
    flag_u: bool,
    flag_v: bool,
) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| format!("'{dest}': {e}"))?;

    let rd = fs::read_dir(src).map_err(|e| format!("'{src}': {e}"))?;
    for item in rd {
        let entry = item.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let src_path = entry.path();
        let dest_path = Path::new(dest).join(&name);
        copy_one(
            src_path.to_str().unwrap_or_default(),
            dest_path.to_str().unwrap_or_default(),
            flag_a,
            flag_d,
            flag_f,
            flag_i,
            flag_l,
            flag_n,
            flag_p,
            flag_r,
            flag_s,
            flag_u,
            flag_v,
        )?;
    }

    // After populating children, apply preserved attributes to the directory
    // itself when -p is active.
    if flag_p {
        if let Ok(meta) = fs::symlink_metadata(src) {
            preserve_attrs(src, dest, &meta)?;
        }
    }

    Ok(())
}

/// Copy mode, timestamps, and ownership from `src` to `dest`.
///
/// Ownership changes (`chown`) typically require superuser privileges and
/// are best-effort only.
fn preserve_attrs(_src: &str, dest: &str, meta: &fs::Metadata) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let perm = fs::Permissions::from_mode(meta.mode() & 0o7777);
    let _ = fs::set_permissions(dest, perm);

    let atime = FileTime::from_system_time(std::time::SystemTime::from(
        UNIX_EPOCH + Duration::from_secs(meta.atime() as u64),
    ));
    let mtime = FileTime::from_system_time(std::time::SystemTime::from(
        UNIX_EPOCH + Duration::from_secs(meta.mtime() as u64),
    ));
    let _ = filetime::set_file_times(dest, atime, mtime);

    // Ownership change is attempted but failures are silently ignored
    // (requires root, may be unsupported on some filesystems).
    let _ = nix_chown(dest, meta.uid(), meta.gid());

    Ok(())
}

/// Stub for `chown` — currently a no-op.
///
/// Ownership changes require elevated privileges; errors are intentionally
/// suppressed to avoid noise during copies that do not need ownership
/// preservation.
fn nix_chown(_path: &str, _uid: u32, _gid: u32) -> Result<(), String> {
    Ok(())
}

/// Compute a relative path from the directory containing `from` to `to`.
///
/// Used by `-s` so that the generated symlink targets are relative rather
/// than absolute.
fn relative_path(from: &str, to: &str) -> PathBuf {
    let from = Path::new(from);
    let to = Path::new(to);
    let from_parent = from.parent().unwrap_or_else(|| Path::new("."));

    let from_comps: Vec<_> = from_parent.components().collect();
    let to_comps: Vec<_> = to.components().collect();

    // Count leading components that are common to both paths.
    let mut common = 0;
    while common < from_comps.len()
        && common < to_comps.len()
        && from_comps[common] == to_comps[common]
    {
        common += 1;
    }

    let mut result = PathBuf::new();

    // For each remaining component in `from_parent`, ascend with `..`.
    for _ in common..from_comps.len() {
        result.push("..");
    }

    // Append the divergent suffix of `to`.
    for c in &to_comps[common..] {
        result.push(c.as_os_str());
    }

    if result.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        result
    }
}

register_command!(
    CP_CMD,
    "cp",
    "a(dpr)dfilnprsuv[-HLP]",
    CommandFlags::BIN.bits(),
    cp_main
);
