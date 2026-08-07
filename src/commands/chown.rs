// =============================================================================
// chown — Change file owner and group.
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Supported options:
//   -R      Recursively change files and directories.
//   -h      Change symlinks themselves rather than the files they point to.
//   -v      Verbose: print each file as it is processed.
//   -f      Suppress error messages for files that cannot be changed.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::ffi::CStr;
use std::fs;
use std::os::unix::fs::{chown, lchown, MetadataExt};
use std::path::Path;

/// Entry point for `chown`.
fn chown_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "Rhvf") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("chown: {e}");
            return 1;
        }
    };

    let flag_R = opts.count('R') > 0;
    let flag_h = opts.count('h') > 0;
    let flag_v = opts.count('v') > 0;
    let flag_f = opts.count('f') > 0;

    // At least one arg: owner[:group] and one file.
    if ctx.optargs.len() < 2 {
        eprintln!("chown: missing operand");
        return 1;
    }

    let owner_spec = ctx.optargs[0].clone();
    let files = &ctx.optargs[1..];

    // Parse owner[:group]
    let (owner_str, group_str) = parse_owner_group(&owner_spec);

    // Resolve UID and GID
    let (uid, gid) = match resolve_ids(owner_str, group_str) {
        Ok(ids) => ids,
        Err(e) => {
            eprintln!("chown: {e}");
            return 1;
        }
    };

    let mut exit_code = 0;

    for file in files {
        if flag_R {
            if let Err(e) = chown_recursive(file, uid, gid, flag_h, flag_v, flag_f) {
                if !flag_f {
                    eprintln!("chown: {e}");
                    exit_code = 1;
                }
            }
        } else {
            if let Err(e) = chown_one(file, uid, gid, flag_h, flag_v, flag_f) {
                if !flag_f {
                    eprintln!("chown: {e}");
                    exit_code = 1;
                }
            }
        }
    }

    exit_code
}

/// Parse the `owner[:group]` specification.
/// Returns (owner_part, group_part), where either may be empty.
fn parse_owner_group(spec: &str) -> (Option<String>, Option<String>) {
    if let Some(idx) = spec.find(':') {
        let owner = if idx == 0 { None } else { Some(spec[..idx].to_string()) };
        let group = if idx + 1 < spec.len() {
            Some(spec[idx + 1..].to_string())
        } else {
            None
        };
        (owner, group)
    } else {
        // Only owner given
        (Some(spec.to_string()), None)
    }
}

/// Resolve owner and group names to numeric UID/GID.
/// Returns `(uid, gid)` where uid or gid may be `None` if not specified.
/// For group, if not specified, we use `None` (meaning don't change group).
fn resolve_ids(
    owner: Option<String>,
    group: Option<String>,
) -> Result<(Option<u32>, Option<u32>), String> {
    let uid = if let Some(name) = owner {
        Some(resolve_user(&name)?)
    } else {
        None
    };

    let gid = if let Some(name) = group {
        Some(resolve_group(&name)?)
    } else {
        None
    };

    Ok((uid, gid))
}

/// Resolve a user name or numeric UID.
fn resolve_user(name: &str) -> Result<u32, String> {
    if let Ok(num) = name.parse::<u32>() {
        return Ok(num);
    }
    // Lookup via getpwnam
    let c_name = std::ffi::CString::new(name).map_err(|_| "invalid user name")?;
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = vec![0u8; 4096];
    let mut pw_ptr: *mut libc::passwd = std::ptr::null_mut();
    let ret = unsafe {
        libc::getpwnam_r(
            c_name.as_ptr(),
            &mut pw,
            buf.as_mut_ptr() as *mut libc::c_char,
            buf.len(),
            &mut pw_ptr,
        )
    };
    if ret != 0 || pw_ptr.is_null() {
        return Err(format!("unknown user '{}'", name));
    }
    Ok(pw.pw_uid)
}

/// Resolve a group name or numeric GID.
fn resolve_group(name: &str) -> Result<u32, String> {
    if let Ok(num) = name.parse::<u32>() {
        return Ok(num);
    }
    let c_name = std::ffi::CString::new(name).map_err(|_| "invalid group name")?;
    let mut gr: libc::group = unsafe { std::mem::zeroed() };
    let mut buf = vec![0u8; 4096];
    let mut gr_ptr: *mut libc::group = std::ptr::null_mut();
    let ret = unsafe {
        libc::getgrnam_r(
            c_name.as_ptr(),
            &mut gr,
            buf.as_mut_ptr() as *mut libc::c_char,
            buf.len(),
            &mut gr_ptr,
        )
    };
    if ret != 0 || gr_ptr.is_null() {
        return Err(format!("unknown group '{}'", name));
    }
    Ok(gr.gr_gid)
}

/// Change ownership of a single file/directory.
/// If `follow_symlink` is false, uses lchown (affects symlink itself).
fn chown_one(
    path: &str,
    uid: Option<u32>,
    gid: Option<u32>,
    follow_symlink: bool,
    verbose: bool,
    force: bool,
) -> Result<(), String> {
    // Check if path exists
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) => {
            if !force {
                return Err(format!("cannot access '{}': {}", path, e));
            }
            return Ok(());
        }
    };

    let result = if follow_symlink {
        // follow symlink -> chown on the target
        chown(path, uid, gid)
    } else {
        // -h -> lchown
        lchown(path, uid, gid)
    };

    match result {
        Ok(()) => {
            if verbose {
                eprintln!("changed ownership of '{}'", path);
            }
            Ok(())
        }
        Err(e) => {
            if !force {
                Err(format!("cannot change ownership of '{}': {}", path, e))
            } else {
                Ok(())
            }
        }
    }
}

/// Recursively change ownership for all files under a directory.
fn chown_recursive(
    root: &str,
    uid: Option<u32>,
    gid: Option<u32>,
    follow_symlink: bool,
    verbose: bool,
    force: bool,
) -> Result<(), String> {
    // First, change the root itself.
    chown_one(root, uid, gid, follow_symlink, verbose, force)?;

    // Then traverse children.
    let meta = fs::symlink_metadata(root).map_err(|e| format!("cannot stat '{}': {}", root, e))?;
    if !meta.is_dir() {
        return Ok(());
    }

    let rd = fs::read_dir(root).map_err(|e| format!("cannot read directory '{}': {}", root, e))?;
    for entry in rd {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let path_str = path.to_str().unwrap_or_default();
        // Recurse into subdirectories
        let child_meta = fs::symlink_metadata(&path)
            .map_err(|e| format!("cannot stat '{}': {}", path_str, e))?;
        if child_meta.is_dir() {
            chown_recursive(path_str, uid, gid, follow_symlink, verbose, force)?;
        } else {
            chown_one(path_str, uid, gid, follow_symlink, verbose, force)?;
        }
    }

    Ok(())
}

register_command!(
    CHOWN_CMD,
    "chown",
    "Rhvf",
    CommandFlags::BIN.bits(),
    chown_main,
    description = "Change file owner and group",
    help = "\
OPTIONS:
-R      Recursively change files and directories.
-h      Change symlinks themselves rather than the files they point to.
-v      Verbose: print each file as it is processed.
-f      Suppress error messages for files that cannot be changed.

USAGE:
  chown [OPTIONS] OWNER[:GROUP] FILE...
  chown [OPTIONS] :GROUP FILE...       (change group only)
  chown [OPTIONS] OWNER: FILE...       (change owner only, group untouched)
"
);