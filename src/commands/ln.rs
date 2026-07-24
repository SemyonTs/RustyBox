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
use std::path::{Component, Path, PathBuf};

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
    let flag_T = opts.count('T') > 0;
    let flag_v = opts.count('v') > 0;
    let target_dir = opts.get_str('t').unwrap_or("");

    let n = ctx.optargs.len();
    if n == 0 {
        return 0; // Should not happen due to "<1", but be safe.
    }

    // -T: the destination must be treated as a regular file, limiting the
    // argument count to two.
    if flag_T && n > 2 {
        eprintln!("ln: with -T at most 2 arguments are allowed");
        return 1;
    }

    // Determine the destination and sources without cloning the entire vector.
    let (sources, dest): (&[String], &str) = if !target_dir.is_empty() {
        // -t DIR: all args are sources, DIR is the destination.
        (&ctx.optargs[..], target_dir)
    } else {
        // Last arg is destination, preceding are sources.
        (&ctx.optargs[..n - 1], ctx.optargs[n - 1].as_str())
    };

    // Decide whether the destination is an existing directory.
    let dest_is_dir = if flag_n || flag_T {
        false
    } else {
        fs::metadata(dest).map(|m| m.is_dir()).unwrap_or(false)
    };

    let mut exit_code: u8 = 0;

    // Pre-allocate reusable buffers for path construction.
    let mut new_path = String::with_capacity(256);
    let mut rel_buf = String::with_capacity(256);

    for src in sources {
        // When the destination is a directory the link is placed inside it
        // using the source's base name.
        new_path.clear();
        if dest_is_dir {
            let base = Path::new(src)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(src);
            new_path.push_str(dest);
            new_path.push('/');
            new_path.push_str(base);
        } else {
            new_path.push_str(dest);
        };

        // Resolve the link target: either the source itself or a relative
        // path from the new link's location.
        let target: &str = if flag_r {
            rel_buf.clear();
            if let Some(r) = relative_path_into(&new_path, src, &mut rel_buf) {
                r
            } else {
                eprintln!("ln: cannot create relative link for '{}'", src);
                exit_code = 1;
                continue;
            }
        } else {
            src.as_str()
        };

        // -f: remove any existing destination entry before linking.
        if flag_f {
            let _ = fs::remove_file(&new_path);
            let _ = fs::remove_dir(&new_path);
        }

        let rc = if flag_s {
            symlink(target, &new_path).is_err()
        } else {
            fs::hard_link(target, &new_path).is_err()
        };

        if rc {
            eprintln!(
                "ln: cannot create {} link from '{}' to '{}'",
                if flag_s { "symbolic" } else { "hard" },
                target,
                new_path
            );
            exit_code = 1;
        } else if flag_v {
            eprintln!("'{}' -> '{}'", new_path, target);
        }
    }

    exit_code
}

/// Compute a relative path from the directory that would contain `from`
/// (the new link) to `to` (the link target), writing the result into `out`.
///
/// Returns `Some(&str)` pointing into `out` on success, `None` on failure.
fn relative_path_into<'a>(from: &str, to: &str, out: &'a mut String) -> Option<&'a str> {
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

    out.clear();

    // Ascend out of the remaining `from_parent` components.
    for _ in common..from_comps.len() {
        if !out.is_empty() {
            out.push('/');
        }
        out.push_str("..");
    }

    // Append the divergent suffix of the target.
    for c in &to_comps[common..] {
        if !out.is_empty() {
            out.push('/');
        }
        match c {
            Component::Normal(s) => out.push_str(s.to_str().unwrap_or("")),
            _ => out.push_str(&c.as_os_str().to_string_lossy()),
        }
    }

    if out.is_empty() {
        out.push('.');
    }

    Some(out.as_str())
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
            Component::CurDir => {}
            Component::ParentDir => {
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
