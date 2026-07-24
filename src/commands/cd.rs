// =============================================================================
// cd — Change the current working directory.
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Supported options:
//   -L   Follow logical path (default).
//   -P   Use physical path (resolve symlinks).
//   -    Change to previous working directory ($OLDPWD).
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Entry point for the `cd` builtin.
///
/// If no directory is given, changes to `$HOME`. If the directory is `-`,
/// changes to `$OLDPWD` and prints the new path.
///
/// POSIX behaviour (IEEE Std 1003.1-2017):
///   - `CDPATH` is searched for relative paths that do not start with `.`
///     and are not absolute.
///   - `-L` (default): $PWD is set to the logical path.
///   - `-P`: $PWD is set to the physical path (symlinks resolved).
fn cd_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "LP") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("cd: {e}");
            return 1;
        }
    };

    let flag_P = opts.count('P') > 0;

    // Determine the target directory.
    let (dir_to_go, print_path) = if ctx.optargs.is_empty() {
        // No arguments -> $HOME.
        match env::var("HOME") {
            Ok(home) => (home, false),
            Err(_) => {
                eprintln!("cd: HOME not set");
                return 1;
            }
        }
    } else if ctx.optargs.len() == 1 {
        let arg = &ctx.optargs[0];
        if arg == "-" {
            // "cd -" -> $OLDPWD.
            match env::var("OLDPWD") {
                Ok(old) => (old, true),
                Err(_) => {
                    eprintln!("cd: OLDPWD not set");
                    return 1;
                }
            }
        } else {
            // Normal directory argument.
            (arg.to_string(), false)
        }
    } else {
        eprintln!("cd: too many arguments");
        return 1;
    };

    // Resolve the directory: try CDPATH for relative, non-dot paths.
    let resolved = resolve_cdpath(&dir_to_go);

    // Compute the new path according to -P (physical) or default logical mode.
    let new_path = if flag_P {
        match fs::canonicalize(&resolved) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("cd: {}: {}", dir_to_go, e);
                return 1;
            }
        }
    } else {
        let p = Path::new(&resolved);
        if p.is_absolute() {
            normalize_path(p)
        } else {
            let current = match env::current_dir() {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("cd: cannot get current directory: {}", e);
                    return 1;
                }
            };
            normalize_path(&current.join(p))
        }
    };

    // For "cd -", print the destination directory (POSIX behaviour).
    if print_path {
        println!("{}", new_path.display());
    }

    // Save the old PWD for the OLDPWD environment variable.
    let old_pwd = env::var("PWD").unwrap_or_else(|_| {
        env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default()
    });

    // Perform the directory change.
    if let Err(e) = env::set_current_dir(&new_path) {
        eprintln!("cd: {}: {}", new_path.display(), e);
        return 1;
    }

    // Update the PWD and OLDPWD environment variables (best-effort).
    // set_var is unsafe in multi-threaded contexts; cd is a builtin that runs
    // in the main thread before any fork, so this is safe in practice.
    unsafe {
        env::set_var("OLDPWD", old_pwd);
        env::set_var("PWD", new_path.to_string_lossy().into_owned());
    }

    0
}

/// Resolve `dir` using `$CDPATH` if applicable (POSIX).
///
/// If `dir` is absolute, starts with `.`, or `CDPATH` is unset/empty,
/// `dir` is returned as-is.  Otherwise each entry in `CDPATH` (colon-
/// separated) is prepended to `dir`; the first resulting path that
/// exists as a directory is used.
///
/// Falls back to the original `dir` if no `CDPATH` entry yields a
/// valid directory.
fn resolve_cdpath(dir: &str) -> String {
    let p = Path::new(dir);

    // Absolute path or starts with "." — CDPATH is not consulted.
    if p.is_absolute() || p.starts_with(".") {
        return dir.to_string();
    }

    let cdpath = match env::var("CDPATH") {
        Ok(v) => v,
        Err(_) => return dir.to_string(),
    };

    if cdpath.is_empty() {
        return dir.to_string();
    }

    for entry in cdpath.split(':') {
        if entry.is_empty() {
            // Empty entry in CDPATH stands for the current directory.
            if Path::new(dir).is_dir() {
                return dir.to_string();
            }
            continue;
        }

        let candidate = Path::new(entry).join(dir);
        if candidate.is_dir() {
            return candidate.to_string_lossy().into_owned();
        }
    }

    // No CDPATH entry matched — POSIX says fall back to original.
    dir.to_string()
}

/// Normalize a path without touching the filesystem:
/// remove `.` and `..` components, keep symlinks unresolved.
fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::CurDir => {}
            _ => normalized.push(component),
        }
    }
    // For absolute paths, ensure we don't end up with an empty path
    // (e.g., "/.." becomes empty after popping).
    if normalized.as_os_str().is_empty() && path.is_absolute() {
        normalized.push("/");
    }
    normalized
}

register_command!(
    CD_CMD,
    "cd",
    "LP",
    CommandFlags::BIN.bits() | CommandFlags::USR.bits(),
    cd_main
);
