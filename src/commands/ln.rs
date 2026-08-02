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
//   -L      Dereference targets that are symbolic links (for hard links).
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Component, Path, PathBuf};

/// Entry point for the `ln` builtin.
///
/// The option string `"<1rt:TvnfsL"` requires at least one positional argument.
fn ln_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "<1rt:TvnfsL") {
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
    let flag_L = opts.count('L') > 0; // Dereference symlinks for hard links
    let target_dir = opts.get_str('t').unwrap_or("");

    let n = ctx.optargs.len();
    if n == 0 {
        return 0;
    }

    if flag_T && n > 2 {
        eprintln!("ln: with -T at most 2 arguments are allowed");
        return 1;
    }

    let (sources, dest): (&[String], &str) = if !target_dir.is_empty() {
        (&ctx.optargs[..], target_dir)
    } else {
        (&ctx.optargs[..n - 1], ctx.optargs[n - 1].as_str())
    };

    let dest_is_dir = if flag_n || flag_T {
        false
    } else {
        fs::metadata(dest).map(|m| m.is_dir()).unwrap_or(false)
    };

    let mut exit_code: u8 = 0;
    let mut new_path = String::with_capacity(256);
    let mut rel_buf = String::with_capacity(256);

    for src in sources {
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

        // Determine the actual source path to link to
        let mut effective_src = src.as_str();

        // If -L is set and we are creating a hard link, dereference the source
        if flag_L && !flag_s {
            if let Ok(canonical) = fs::canonicalize(src) {
                // We need to store this somewhere or use it directly.
                // Since canonicalize returns PathBuf, we can't easily borrow it
                // without storing it. For simplicity in this loop, we might
                // need a small buffer or just use the original if canonicalization fails.
                // Note: This requires changing the loop structure slightly or using a temp var.
                // For now, let's assume we pass the canonical path to the link function.
                // But `symlink` and `hard_link` take &str or AsRef<Path>.

                // Let's refine: we will use `effective_src` only for relative calc.
                // For the actual link call, we use the resolved path.
            }
        }

        let target: &str = if flag_r {
            rel_buf.clear();
            // For relative links, we usually want the relative path from dest to src
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

        // -f: remove existing destination
        if flag_f {
            // Try to remove as file first, then as dir/symlink-to-dir
            if fs::remove_file(&new_path).is_err() {
                let _ = fs::remove_dir(&new_path);
            }
        }

        let rc = if flag_s {
            symlink(target, &new_path).is_err()
        } else {
            // For hard links, if -L is specified, resolve the source first
            let link_source = if flag_L {
                // We need an owned String here if we canonicalize
                match fs::canonicalize(src) {
                    Ok(p) => p.to_string_lossy().to_string(),
                    Err(_) => src.clone(),
                }
            } else {
                src.clone()
            };

            fs::hard_link(&link_source, &new_path).is_err()
        };

        if rc {
            eprintln!(
                "ln: cannot create {} link from '{}' to '{}'",
                if flag_s { "symbolic" } else { "hard" },
                src,
                new_path
            );
            exit_code = 1;
        } else if flag_v {
            eprintln!(
                "'{}' -> '{}'",
                new_path,
                if flag_r { rel_buf.as_str() } else { src }
            );
        }
    }

    exit_code
}

/// Compute a relative path from the directory that would contain `from`
/// (the new link) to `to` (the link target).
fn relative_path_into<'a>(from: &str, to: &str, out: &'a mut String) -> Option<&'a str> {
    let from_parent = Path::new(from).parent().unwrap_or_else(|| Path::new("."));

    // Use lightweight canonicalization for relative path calculation
    let from_can = canonicalize_light(from_parent.to_str().unwrap_or("."));
    let to_can = canonicalize_light(to);

    let from_comps: Vec<_> = from_can.components().collect();
    let to_comps: Vec<_> = to_can.components().collect();

    let mut common = 0;
    while common < from_comps.len()
        && common < to_comps.len()
        && from_comps[common] == to_comps[common]
    {
        common += 1;
    }

    out.clear();

    for _ in common..from_comps.len() {
        if !out.is_empty() {
            out.push('/');
        }
        out.push_str("..");
    }

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
    "<1rt:TvnfsL",
    CommandFlags::BIN.bits(),
    ln_main
);
