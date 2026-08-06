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
//   -f      Force overwrite of existing destination files (regular files only).
//   -i      Prompt before overwriting.
//   -l      Create hard links instead of copying.
//   -n      Do not overwrite an existing file (no-clobber).
//   -p      Preserve mode, ownership, and timestamps. Implied by -a.
//   -R, -r  Copy directories recursively. Implied by -a.
//   -s      Create symbolic links instead of copying.
//   -u      Copy only when the source file is newer than the destination.
//   -v      Verbose: print each source/destination pair.
//   -H      Follow command-line symlinks (recursive only).
//   -L      Follow all symlinks (recursive only).
//   -P      Never follow symlinks (default for recursive).
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use filetime::FileTime;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::symlink;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

// Raw FFI declarations for system calls not directly available in std.
mod ffi {
    use std::os::raw::{c_char, c_int};
    pub type mode_t = u32;
    pub type uid_t = u32;
    pub type gid_t = u32;

    unsafe extern "C" {
        pub fn umask(mask: mode_t) -> mode_t;
        pub fn chown(path: *const c_char, owner: uid_t, group: gid_t) -> c_int;
    }
}

/// Symlink handling policy for recursive copies.
#[derive(Copy, Clone, PartialEq)]
enum SymlinkPolicy {
    /// Never follow symlinks (copy the link itself).
    Preserve,
    /// Follow symlinks given as command-line operands; preserve during traversal.
    FollowArgs,
    /// Follow all symlinks encountered.
    FollowAll,
}

/// Entry point for the `cp` builtin.
fn cp_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "a(dpr)dfilnprsuvR[-HLP]") {
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

    // Symlink handling flags (mutually exclusive, last specified wins).
    let flag_H = opts.count('H') > 0;
    let flag_L = opts.count('L') > 0;
    let flag_P = opts.count('P') > 0;

    // Determine symlink policy for recursive copies.
    // For non‑recursive copies, symlink behaviour is controlled by -d / -P alone
    // and the policy is irrelevant.  For recursive, the last of -H, -L, -P wins,
    // defaulting to Preserve.
    let symlink_policy = if flag_H {
        SymlinkPolicy::FollowArgs
    } else if flag_L {
        SymlinkPolicy::FollowAll
    } else {
        // -P or nothing (default) -> preserve
        SymlinkPolicy::Preserve
    };

    let n = ctx.optargs.len();
    if n < 2 {
        eprintln!("cp: not enough arguments");
        return 1;
    }

    let sources = &ctx.optargs[..n - 1];
    let dest = &ctx.optargs[n - 1];

    // Check whether the destination already exists as a directory.
    let dest_is_dir = if sources.len() > 1 {
        // POSIX: with multiple sources, target must be an existing directory.
        match fs::metadata(dest) {
            Ok(meta) => meta.is_dir(),
            Err(_) => {
                eprintln!("cp: target '{}' is not a directory", dest);
                return 1;
            }
        }
    } else {
        fs::metadata(dest).map(|m| m.is_dir()).unwrap_or(false)
    };

    // For a single source that is a regular file (or symlink to one) where the
    // destination path does not name an existing directory, we may need to create
    // parent directories (e.g., `cp file /new/path/to/file`).
    if !dest_is_dir && sources.len() == 1 {
        if let Some(parent) = Path::new(dest).parent() {
            if !parent.exists() {
                if let Err(e) = fs::create_dir_all(parent) {
                    eprintln!("cp: cannot create directory '{}': {}", parent.display(), e);
                    return 1;
                }
            }
        }
    }

    let mut exit_code: u8 = 0;

    for src in sources {
        // Destination path for this source.
        let target = if dest_is_dir {
            let file_name = Path::new(src)
                .file_name()
                .unwrap_or_else(|| std::ffi::OsStr::new("unnamed"));
            Path::new(dest).join(file_name)
        } else {
            PathBuf::from(dest)
        };

        if let Err(e) = copy_one(
            src,
            target.to_str().unwrap_or_default(),
            true, // is_top_level
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
            symlink_policy,
        ) {
            eprintln!("cp: {e}");
            exit_code = 1;
        }
    }

    exit_code
}

/// Dispatch a single source/destination pair to the appropriate copy strategy.
fn copy_one(
    src: &str,
    dest: &str,
    is_top_level: bool,
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
    symlink_policy: SymlinkPolicy,
) -> Result<(), String> {
    // ------------------------------------------------------------------
    // Resolve source metadata, taking symlink policy into account.
    // ------------------------------------------------------------------
    let symlink_meta =
        fs::symlink_metadata(src).map_err(|e| format!("cannot stat '{}': {}", src, e))?;

    let (meta, should_preserve_symlink) = if symlink_meta.file_type().is_symlink() {
        // For non‑recursive copies, the decision is simple:
        //   - -d or -P → preserve the symlink itself.
        //   - otherwise → follow (dereference).
        if !flag_r {
            let preserve = flag_d; // -d preserves symlinks
            (
                if preserve {
                    symlink_meta.clone()
                } else {
                    fs::metadata(src).map_err(|e| format!("cannot stat '{}': {}", src, e))?
                },
                preserve,
            )
        } else {
            // Recursive copy: obey the SymlinkPolicy.
            let follow = match symlink_policy {
                SymlinkPolicy::Preserve => false,
                SymlinkPolicy::FollowArgs => is_top_level, // follow only command‑line sources
                SymlinkPolicy::FollowAll => true,
            };
            if follow {
                (
                    fs::metadata(src).map_err(|e| format!("cannot stat '{}': {}", src, e))?,
                    false,
                )
            } else {
                (symlink_meta.clone(), true)
            }
        }
    } else {
        (symlink_meta, false)
    };

    // ------------------------------------------------------------------
    // Handle overwrite protection and prompts.
    // ------------------------------------------------------------------
    let dest_exists = fs::symlink_metadata(dest).is_ok();

    // -n (no‑clobber): silently skip if destination already exists.
    if flag_n && dest_exists {
        return Ok(());
    }

    // -u (update): skip if destination has an equal or newer modification time.
    if flag_u && dest_exists {
        let src_mtime = meta.modified().ok();
        let dest_mtime = fs::metadata(dest).ok().and_then(|m| m.modified().ok());
        if let (Some(sm), Some(dm)) = (src_mtime, dest_mtime) {
            if dm >= sm {
                return Ok(());
            }
        }
    }

    // -f (force): attempt to remove an existing destination *only* if it is
    // not a directory.  POSIX applies -f only to regular file copies.
    if flag_f && dest_exists {
        let dest_meta =
            fs::symlink_metadata(dest).map_err(|e| format!("cannot stat '{}': {}", dest, e))?;
        if !dest_meta.is_dir() {
            fs::remove_file(dest).map_err(|e| format!("cannot remove '{}': {}", dest, e))?;
            // After removal, destination no longer exists.
        }
        // If dest is a directory, -f does nothing (the directory branch will
        // handle errors later).
    }

    // Re‑check existence after possible -f removal.
    let dest_exists = fs::symlink_metadata(dest).is_ok();

    // -i (interactive): prompt before overwriting an existing non‑directory.
    if flag_i && dest_exists && !flag_f {
        eprint!("cp: overwrite '{}'? ", dest);
        io::stdout().flush().map_err(|e| e.to_string())?;
        let mut buf = String::new();
        io::stdin().read_line(&mut buf).map_err(|e| e.to_string())?;
        if !buf.trim().eq_ignore_ascii_case("y") {
            return Ok(());
        }
    }

    // ------------------------------------------------------------------
    // Verbose output.
    // ------------------------------------------------------------------
    if flag_v {
        eprintln!("'{}' -> '{}'", src, dest);
    }

    // ------------------------------------------------------------------
    // Copy a symlink as a symlink (when policy says so).
    // ------------------------------------------------------------------
    if should_preserve_symlink {
        let target = fs::read_link(src).map_err(|e| format!("cannot readlink '{}': {}", src, e))?;
        symlink(target, dest).map_err(|e| format!("cannot create symlink '{}': {}", dest, e))?;
        return Ok(());
    }

    // ------------------------------------------------------------------
    // Directory copy.
    // ------------------------------------------------------------------
    if meta.is_dir() {
        if !flag_r {
            return Err(format!("omitting directory '{}'", src));
        }
        copy_dir(
            src,
            dest,
            is_top_level, // not used inside copy_dir; children will have false
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
            symlink_policy,
        )?;
        return Ok(());
    }

    // ------------------------------------------------------------------
    // Hard link creation (overrides regular copy).
    // ------------------------------------------------------------------
    if flag_l {
        fs::hard_link(src, dest)
            .map_err(|e| format!("cannot create hard link '{}': {}", dest, e))?;
        return Ok(());
    }

    // ------------------------------------------------------------------
    // Symbolic link creation (source path used as link target).
    // ------------------------------------------------------------------
    if flag_s {
        symlink(src, dest).map_err(|e| format!("cannot create symlink '{}': {}", dest, e))?;
        return Ok(());
    }

    // ------------------------------------------------------------------
    // Regular file copy.
    // ------------------------------------------------------------------
    fs::copy(src, dest).map_err(|e| format!("cannot copy '{}' to '{}': {}", src, dest, e))?;

    // Apply final permissions (and optionally timestamps/ownership).
    set_file_attrs(dest, &meta, flag_p)?;

    Ok(())
}

/// Recursively copy a directory tree from `src` to `dest`.
///
/// The destination directory is created if it does not exist.  Existing
/// files are overwritten according to the flags.
fn copy_dir(
    src: &str,
    dest: &str,
    _is_top_level: bool,
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
    symlink_policy: SymlinkPolicy,
) -> Result<(), String> {
    // Resolve the real metadata of the source directory (following symlinks
    // according to policy, though for the top‑level directory it should
    // already be handled).
    let src_meta = fs::metadata(src).map_err(|e| format!("cannot access '{}': {}", src, e))?;
    if !src_meta.is_dir() {
        return Err(format!("'{}' is not a directory", src));
    }

    let src_mode = src_meta.mode();

    // Obtain current file creation mask.
    let umask = get_umask();

    // Create destination directory if it does not exist or is not a directory.
    match fs::symlink_metadata(dest) {
        Ok(meta) => {
            if !meta.is_dir() {
                return Err(format!("'{}' exists but is not a directory", dest));
            }
            // Directory already exists; nothing to create.
        }
        Err(_) => {
            // Create with permissions = (source_mode & ~umask) | S_IRWXU
            // so that we can populate it.
            let init_mode = (src_mode & !umask) | 0o700;
            fs::create_dir(dest)
                .map_err(|e| format!("cannot create directory '{}': {}", dest, e))?;
            fs::set_permissions(dest, fs::Permissions::from_mode(init_mode))
                .map_err(|e| format!("cannot set permissions on '{}': {}", dest, e))?;
        }
    }

    // Process every entry in the source directory.
    let rd = fs::read_dir(src).map_err(|e| format!("cannot read directory '{}': {}", src, e))?;
    for item in rd {
        let entry = item.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let src_path = entry.path();
        let dest_path = Path::new(dest).join(&name);

        copy_one(
            src_path.to_str().unwrap_or_default(),
            dest_path.to_str().unwrap_or_default(),
            false, // not top‑level
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
            symlink_policy,
        )?;
    }

    // After children are copied, set the final permissions on the directory.
    let final_mode = if flag_p {
        src_mode & 0o7777
    } else {
        (src_mode & !umask) & 0o7777
    };
    fs::set_permissions(dest, fs::Permissions::from_mode(final_mode))
        .map_err(|e| format!("cannot set permissions on '{}': {}", dest, e))?;

    // Preserve timestamps and ownership when -p is given.
    if flag_p {
        preserve_extra_attrs(dest, &src_meta)?;
    }

    Ok(())
}

/// Set permissions (and, if -p, timestamps and ownership) on a regular file
/// or symlink destination.
fn set_file_attrs(dest: &str, src_meta: &fs::Metadata, flag_p: bool) -> Result<(), String> {
    let umask = get_umask();
    let src_mode = src_meta.mode();

    let mode = if flag_p {
        src_mode & 0o7777
    } else {
        (src_mode & !umask) & 0o7777
    };

    fs::set_permissions(dest, fs::Permissions::from_mode(mode))
        .map_err(|e| format!("cannot set permissions on '{}': {}", dest, e))?;

    if flag_p {
        preserve_extra_attrs(dest, src_meta)?;
    }

    Ok(())
}

/// Duplicate timestamps and ownership from source metadata onto destination.
/// If ownership cannot be duplicated, the set‑user‑ID and set‑group‑ID bits
/// are cleared in accordance with POSIX.
fn preserve_extra_attrs(dest: &str, src_meta: &fs::Metadata) -> Result<(), String> {
    // Timestamps.
    let atime =
        FileTime::from_system_time(UNIX_EPOCH + Duration::from_secs(src_meta.atime() as u64));
    let mtime =
        FileTime::from_system_time(UNIX_EPOCH + Duration::from_secs(src_meta.mtime() as u64));
    filetime::set_file_times(dest, atime, mtime)
        .map_err(|e| format!("cannot set timestamps on '{}': {}", dest, e))?;

    // Ownership.
    let chown_res = do_chown(dest, src_meta.uid(), src_meta.gid());
    if chown_res.is_err() {
        // Clear set‑user‑ID and set‑group‑ID bits.
        let cur_mode = fs::symlink_metadata(dest)
            .map_err(|e| format!("cannot stat '{}': {}", dest, e))?
            .mode();
        let new_mode = cur_mode & !0o6000; // S_ISUID | S_ISGID
        fs::set_permissions(dest, fs::Permissions::from_mode(new_mode))
            .map_err(|e| format!("cannot adjust permissions on '{}': {}", dest, e))?;
    }

    Ok(())
}

/// Perform a `chown(path, uid, gid)` system call.
fn do_chown(path: &str, uid: u32, gid: u32) -> Result<(), String> {
    let cpath = std::ffi::CString::new(path).map_err(|e| e.to_string())?;
    let ret = unsafe { ffi::chown(cpath.as_ptr(), uid, gid) };
    if ret != 0 {
        Err(format!(
            "cannot chown '{}' to {}:{}: {}",
            path,
            uid,
            gid,
            io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}

/// Retrieve the current file creation mask.
fn get_umask() -> u32 {
    // The only safe way to read the umask without changing it is to set a
    // temporary mask and restore the original.
    let saved = unsafe { ffi::umask(0o777) };
    unsafe { ffi::umask(saved) };
    saved
}

/// Compute a relative path from the directory containing `from` to `to`.
///
/// Used by `-s` so that the generated symlink targets are relative rather
/// than absolute.  (Currently unused; kept for potential future use.)
fn relative_path(from: &str, to: &str) -> PathBuf {
    let from = Path::new(from);
    let to = Path::new(to);
    let from_parent = from.parent().unwrap_or_else(|| Path::new("."));

    let from_comps: Vec<_> = from_parent.components().collect();
    let to_comps: Vec<_> = to.components().collect();

    let mut common = 0;
    while common < from_comps.len()
        && common < to_comps.len()
        && from_comps[common] == to_comps[common]
    {
        common += 1;
    }

    let mut result = PathBuf::new();
    for _ in common..from_comps.len() {
        result.push("..");
    }
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
    "a(dpr)dfilnprsuvR[-HLP]",
    CommandFlags::BIN.bits(),
    cp_main,
    description = "Copy files and directories",
    help = "\
OPTIONS:
-a      Archive mode: equivalent to -dpR.
-d      Preserve symlinks (do not dereference). Implied by -a.
-f      Force overwrite of existing destination files (regular files only).
-i      Prompt before overwriting.
-l      Create hard links instead of copying.
-n      Do not overwrite an existing file (no-clobber).
-p      Preserve mode, ownership, and timestamps. Implied by -a.
-R, -r  Copy directories recursively. Implied by -a.
-s      Create symbolic links instead of copying.
-u      Copy only when the source file is newer than the destination.
-v      Verbose: print each source/destination pair.
-H      Follow command-line symlinks (recursive only).
-L      Follow all symlinks (recursive only).
-P      Never follow symlinks (default for recursive)."
);
