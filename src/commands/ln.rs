// =============================================================================
// ln — Create links between files.
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
//   -s      Create symbolic links instead of hard links.
//   -f      Force: remove existing destination files before linking.
//   -n      Treat a destination that is a symlink to a directory as a file.
//   -r      Create relative symbolic links. Implies -s.
//   -t DIR  Specify the target directory (all links are created inside DIR).
//   -T      Treat the destination as a normal file, not a directory.
//   -v      Verbose: print the name of each created link.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

/// Entry point for the `ln` builtin.
///
/// The option string `"<1rt:Tvnfs"` requires at least one positional argument.
/// When `-r` is given, `-s` is implied because relative paths only make sense
/// for symbolic links.
fn ln_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "<1rt:Tvnfs") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("ln: {e}");
            return 1;
        }
    };

    let flag_f = opts.count('f') > 0;
    let flag_n = opts.count('n') > 0;
    let flag_r = opts.count('r') > 0;
    let flag_s = opts.count('s') > 0 || flag_r; // -r implies symbolic link.
    let flag_t = opts.count('T') > 0;
    let flag_v = opts.count('v') > 0;
    let target_dir = opts.get_str('t').unwrap_or("");

    let mut args = ctx.optargs.clone();
    if args.is_empty() {
        args.push(".".to_string());
    }

    // -T: the destination must be treated as a regular file, limiting the
    // argument count to two.
    if flag_t && args.len() > 2 {
        eprintln!("ln: with -T at most 2 arguments are allowed");
        return 1;
    }

    // Determine the destination (the last argument, unless -t overrides it).
    let dest = if !target_dir.is_empty() {
        target_dir.to_string()
    } else {
        args.pop().unwrap()
    };

    // Decide whether the destination is an existing directory.
    let dest_is_dir = if flag_n || flag_t {
        false
    } else {
        fs::metadata(&dest).map(|m| m.is_dir()).unwrap_or(false)
    };

    let mut exit_code: u8 = 0;
    let sources: Vec<String> = args;

    for src in &sources {
        // When the destination is a directory the link is placed inside it
        // using the source's base name.
        let new = if dest_is_dir {
            let base = Path::new(src)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(src);
            Path::new(&dest).join(base).to_string_lossy().into_owned()
        } else {
            dest.clone()
        };

        // Resolve the link target: either the source itself or a relative
        // path from the new link's location.
        let target = if flag_r {
            match relative_path(&new, src) {
                Some(r) => r,
                None => {
                    eprintln!("ln: cannot create relative link for '{}'", src);
                    exit_code = 1;
                    continue;
                }
            }
        } else {
            src.clone()
        };

        // -f: remove any existing destination entry before linking.
        if flag_f {
            let _ = fs::remove_file(&new);
            let _ = fs::remove_dir(&new);
        }

        let rc = if flag_s {
            symlink(&target, &new).is_err()
        } else {
            fs::hard_link(&target, &new).is_err()
        };

        if rc {
            eprintln!(
                "ln: cannot create {} link from '{}' to '{}'",
                if flag_s { "symbolic" } else { "hard" },
                target,
                new
            );
            exit_code = 1;
        } else if flag_v {
            eprintln!("'{}' -> '{}'", new, target);
        }
    }

    exit_code
}

/// Compute a relative path from the directory that would contain `from`
/// (the new link) to `to` (the link target).
///
/// Returns `None` if a relative path cannot be constructed.
fn relative_path(from: &str, to: &str) -> Option<String> {
    let from_parent = Path::new(from).parent().unwrap_or_else(|| Path::new("."));
    let from_can = canonicalize_light(from_parent.to_str().unwrap_or("."));
    let to_can = canonicalize_light(to);

    let from_comps: Vec<_> = from_can.components().collect();
    let to_comps: Vec<_> = to_can.components().collect();

    // Find the length of the common prefix.
    let mut common = 0;
    while common < from_comps.len()
        && common < to_comps.len()
        && from_comps[common] == to_comps[common]
    {
        common += 1;
    }

    let mut result = PathBuf::new();

    // Ascend out of the remaining `from_parent` components.
    for _ in common..from_comps.len() {
        result.push("..");
    }

    // Append the divergent suffix of the target.
    for c in &to_comps[common..] {
        result.push(c.as_os_str());
    }

    if result.as_os_str().is_empty() {
        Some(".".to_string())
    } else {
        Some(result.to_string_lossy().into_owned())
    }
}

/// Lightweight path canonicalisation that resolves `.` and `..` segments
/// without touching the filesystem.
///
/// This is sufficient for relative-path computation; symlinks are not
/// resolved.
fn canonicalize_light(p: &str) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in Path::new(p).components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

register_command!(
    LN_CMD,
    "ln",
    "<1rt:Tvnfs",
    CommandFlags::BIN.bits(),
    ln_main
);
