// =============================================================================
// rm — Remove files and directories.
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
//   -f      Force: ignore nonexistent files, never prompt.
//   -i      Prompt before every removal.
//   -r, -R  Remove directories and their contents recursively.
//   -v      Verbose: print the name of each removed entry.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

/// Entry point for the `rm` builtin.
///
/// When `-f` is given zero arguments are permitted (the command silently
/// succeeds).  Explicit attempts to remove `/` or `..` are rejected.
fn rm_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "f(force)iRrv[-fi]") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("rm: {e}");
            return 1;
        }
    };

    let flag_f = opts.count('f') > 0;
    let flag_i = opts.count('i') > 0;
    let flag_r = opts.count('r') > 0 || opts.count('R') > 0;
    let flag_v = opts.count('v') > 0;

    // -f allows zero arguments; otherwise at least one is required.
    if ctx.optargs.is_empty() && !flag_f {
        eprintln!("rm: missing operand");
        return 1;
    }

    let mut exit_code: u8 = 0;
    let stdin = io::stdin();
    let mut out = io::stdout();

    for arg in &ctx.optargs {
        // Refuse to remove the root directory.
        if arg == "/" {
            eprintln!("rm: refusing to remove '/' without --no-preserve-root");
            exit_code = 1;
            continue;
        }

        // Reject paths whose final component is `..` (as toybox does).
        if Path::new(arg)
            .file_name()
            .map(|n| n == "..")
            .unwrap_or(false)
        {
            eprintln!("rm: unsafe path '{}'", arg);
            exit_code = 1;
            continue;
        }

        // -f: silently skip entries that do not exist.
        if flag_f && !path_exists(arg) {
            continue;
        }

        if !remove_path(arg, flag_f, flag_i, flag_r, flag_v, &stdin, &mut out) {
            exit_code = 1;
        }
    }

    let _ = out.flush();
    exit_code
}

/// Return `true` if a filesystem entry exists at `p`.
fn path_exists(p: &str) -> bool {
    Path::new(p).exists() || Path::new(p).symlink_metadata().is_ok()
}

/// Remove a single filesystem entry.
///
/// Directories are handled recursively when `recursive` is true.  Returns
/// `true` on success (or when the user declines an interactive prompt).
fn remove_path(
    path: &str,
    force: bool,
    interactive: bool,
    recursive: bool,
    verbose: bool,
    stdin: &io::Stdin,
    out: &mut impl Write,
) -> bool {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) => {
            if !force {
                eprintln!("rm: cannot remove '{}': {}", path, e);
            }
            return force;
        }
    };

    let is_dir = meta.is_dir();
    let is_symlink = meta.file_type().is_symlink();

    // POSIX forbids removing `.` and `..` even when named explicitly.
    let name = Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    if name == "." || name == ".." {
        if !force {
            eprintln!("rm: cannot remove '{}': it is '.' or '..'", path);
        }
        return force;
    }

    // Interactive prompt for non-directories and symlinks.
    if interactive && !is_dir && !prompt(path, "file", stdin, out) {
        return true; // user declined — not an error
    }

    if is_dir && !is_symlink {
        if !recursive {
            if !force {
                eprintln!("rm: cannot remove '{}': is a directory", path);
            }
            return force;
        }

        // Interactive prompt before descending into a directory (POSIX 2(d)).
        if interactive && !prompt(path, "directory", stdin, out) {
            return true;
        }

        if let Err(e) = remove_dir_all_recursive(path, force, interactive, verbose, stdin, out) {
            if !force {
                eprintln!("rm: cannot remove '{}': {}", path, e);
            }
            return force;
        }

        if verbose {
            let _ = writeln!(out, "rm: removed directory '{}'", path);
        }
        return true;
    }

    // Regular file or symlink.
    if let Err(e) = fs::remove_file(path) {
        if !force {
            eprintln!("rm: cannot remove '{}': {}", path, e);
        }
        return force;
    }

    if verbose {
        let _ = writeln!(out, "rm: removed '{}'", path);
    }

    true
}

/// Recursively delete the contents of a directory, then the directory itself
/// (post-order traversal).
fn remove_dir_all_recursive(
    path: &str,
    force: bool,
    interactive: bool,
    verbose: bool,
    stdin: &io::Stdin,
    out: &mut impl Write,
) -> io::Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let child = entry.path();
        let child_str = child.to_string_lossy().into_owned();

        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => {
                // If metadata is unavailable, skip the entry when forcing;
                // otherwise report the error.
                if !force {
                    return Err(io::Error::new(io::ErrorKind::Other, "permission denied"));
                }
                continue;
            }
        };

        if meta.is_dir() && !meta.file_type().is_symlink() {
            remove_dir_all_recursive(&child_str, force, interactive, verbose, stdin, out)?;
        } else {
            if interactive && !prompt(&child_str, "file", stdin, out) {
                continue;
            }
            fs::remove_file(&child)?;
            if verbose {
                let _ = writeln!(out, "rm: removed '{}'", child_str);
            }
        }
    }

    fs::remove_dir(path)?;
    Ok(())
}

/// Ask the user whether to remove a filesystem entry.
///
/// Returns `true` if the user answered affirmatively.
fn prompt(path: &str, kind: &str, stdin: &io::Stdin, out: &mut impl Write) -> bool {
    let _ = write!(out, "rm: remove {} '{}'? ", kind, path);
    let _ = out.flush();

    let mut line = String::new();
    if stdin.read_line(&mut line).is_err() {
        return false;
    }

    matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

register_command!(
    RM_CMD,
    "rm",
    "f(force)iRrv[-fi]",
    CommandFlags::BIN.bits(),
    rm_main
);
