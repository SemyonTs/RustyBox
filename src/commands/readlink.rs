// =============================================================================
// readlink — Print the target of a symbolic link.
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
//   -e      Canonicalise by following every symlink; fail if the final
//           component is missing.
//   -f      Canonicalise; fail only if a parent directory is missing (the
//           final component may be absent).
//   -m      Canonicalise without touching the filesystem (resolve `..` and
//           `.` components only).
//   -n      Do not output the trailing newline.
//   -q      Quiet: suppress error messages.
//   -z      Delimit output with NUL instead of newline.
//   -v      Verbose: prefix output with the original path.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

/// Entry point for the `readlink` builtin.
///
/// Without `-e`, `-f`, or `-m` the raw symlink target is printed.
fn readlink_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "<1vnf(canonicalize)emqz[-mef][-qv]") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("readlink: {e}");
            return 1;
        }
    };

    let flag_e = opts.count('e') > 0;
    let flag_f = opts.count('f') > 0;
    let flag_m = opts.count('m') > 0;
    let flag_n = opts.count('n') > 0;
    let flag_q = opts.count('q') > 0;
    let flag_z = opts.count('z') > 0;
    let flag_v = opts.count('v') > 0;

    let mut exit_code: u8 = 0;
    let stdout = std::io::stdout();
    let mut writer = BufWriter::new(stdout.lock());

    // Reusable buffer for building output lines.
    let mut out_buf = String::with_capacity(256);

    for file in &ctx.optargs {
        match resolve(file, flag_e, flag_f, flag_m) {
            Ok(resolved) => {
                out_buf.clear();
                if flag_v {
                    out_buf.push_str(file);
                    out_buf.push_str(" -> ");
                }
                out_buf.push_str(&resolved);
                if flag_z {
                    out_buf.push('\0');
                } else if !flag_n {
                    out_buf.push('\n');
                }
                writer.write_all(out_buf.as_bytes()).ok();
            }
            Err(e) => {
                if !flag_q {
                    eprintln!("readlink: '{}': {}", file, e);
                }
                exit_code = 1;
            }
        }
    }

    writer.flush().ok();
    exit_code
}

/// Resolve `path` according to the canonicalisation flags.
///
/// - Without `-e`/`-f`/`-m`: return the literal symlink target.
/// - `-e`: fully resolve via `canonicalize`; every component must exist.
/// - `-f`: fully resolve; only parent directories must exist.
/// - `-m`: resolve `.` and `..` lexically without touching the filesystem.
fn resolve(path: &str, e: bool, f: bool, m: bool) -> Result<String, String> {
    // Default: raw symlink target.
    if !e && !f && !m {
        return fs::read_link(path)
            .map(|p| p.to_string_lossy().into_owned())
            .map_err(|e| e.to_string());
    }

    // Canonicalisation modes.
    if e || f {
        // Both -e and -f require the path to be resolvable up to the last
        // existing component.
        fs::canonicalize(path)
            .map(|p| p.to_string_lossy().into_owned())
            .map_err(|e| e.to_string())
    } else {
        // -m: lexical canonicalisation only.
        Ok(canonicalize_light(path).to_string_lossy().into_owned())
    }
}

/// Lightweight path canonicalisation that resolves `.` and `..` segments
/// without accessing the filesystem.
///
/// Symlinks are not followed; this is equivalent to `realpath -m`.
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
    READLINK_CMD,
    "readlink",
    "<1vnf(canonicalize)emqz[-mef][-qv]",
    CommandFlags::BIN.bits() | CommandFlags::USR.bits(),
    readlink_main
);
