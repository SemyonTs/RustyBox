// =============================================================================
// du — Estimate file space usage.
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
//   -h         Human-readable sizes (e.g. 1K, 234M, 2G).
//   -s         Display only a total for each argument (summary).
//   -a         Show counts for all files, not just directories.
//   -d DEPTH   Limit recursion to DEPTH levels below each argument.
//   -H         Follow symlinks specified on the command line.
//   -k         Write file sizes in 1024-byte blocks.
//   -L         Follow all symlinks.
//   -x         Limit to the same filesystem.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::fs;
use std::os::unix::fs::MetadataExt;

/// Entry point for the `du` builtin.
///
/// When no paths are given the current directory is used by default.
fn du_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "hsad:HkLx") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("du: {e}");
            return 1;
        }
    };

    let flag_h = opts.count('h') > 0;
    let flag_s = opts.count('s') > 0;
    let flag_a = opts.count('a') > 0;
    let flag_k = opts.count('k') > 0;
    let flag_H = opts.count('H') > 0;
    let flag_L = opts.count('L') > 0;
    let flag_x = opts.count('x') > 0;
    let depth = opts.get_int('d').unwrap_or(-1);

    let mut exit_code: u8 = 0;

    if ctx.optargs.is_empty() {
        match du_dir(
            ".",
            flag_h,
            flag_s,
            flag_a,
            flag_k,
            flag_H,
            flag_L,
            flag_x,
            None,
            depth,
            0,
            &mut exit_code,
        ) {
            Err(e) => {
                eprintln!("du: {e}");
                exit_code = 1;
            }
            _ => {}
        }
    } else {
        for dir in &ctx.optargs {
            match du_dir(
                dir,
                flag_h,
                flag_s,
                flag_a,
                flag_k,
                flag_H,
                flag_L,
                flag_x,
                None,
                depth,
                0,
                &mut exit_code,
            ) {
                Err(e) => {
                    eprintln!("du: {e}");
                    exit_code = 1;
                }
                _ => {}
            }
        }
    }

    exit_code
}

/// Recursively accumulate disk usage for a single filesystem entry.
///
/// Returns the total number of blocks allocated to the subtree
/// rooted at `dir`. The size of each node is reported according to the
/// active flags (summary, all-files, depth limit, block size).
fn du_dir(
    dir: &str,
    flag_h: bool,
    flag_s: bool,
    flag_a: bool,
    flag_k: bool,
    flag_H: bool,
    flag_L: bool,
    flag_x: bool,
    root_dev: Option<u64>,
    max_depth: i64,
    cur_depth: i64,
    exit_code: &mut u8,
) -> Result<u64, String> {
    let follow_symlink = flag_L || (flag_H && cur_depth == 0);
    let meta = if follow_symlink {
        fs::metadata(dir).map_err(|e| format!("'{dir}': {e}"))?
    } else {
        fs::symlink_metadata(dir).map_err(|e| format!("'{dir}': {e}"))?
    };

    if flag_x {
        let dev = meta.dev();
        let current_root_dev = root_dev.unwrap_or(dev);
        if cur_depth > 0 && dev != current_root_dev {
            return Ok(0);
        }
    }

    // meta.blocks() always returns 512-byte blocks on Unix.
    let size_512 = meta.blocks();
    // Convert to 1024-byte blocks if -k is specified, rounding up.
    let size = if flag_k { (size_512 + 1) / 2 } else { size_512 };

    if !meta.is_dir() {
        if flag_a || cur_depth == 0 {
            println!("{} {}", format_blocks(size, flag_h, flag_k), dir);
        }
        return Ok(size);
    }

    let mut total = size;

    if !flag_s && (max_depth < 0 || cur_depth < max_depth) {
        let rd = fs::read_dir(dir).map_err(|e| format!("'{dir}': {e}"))?;
        let current_root_dev = if flag_x {
            root_dev.or(Some(meta.dev()))
        } else {
            None
        };

        for item in rd {
            let entry = match item {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("du: {dir}: {e}");
                    *exit_code = 1;
                    continue;
                }
            };
            let path = entry.path();
            let name = entry.file_name();
            let name_str = name.to_str().unwrap_or_default();

            // Skip the self-referential directory entries.
            if name_str == "." || name_str == ".." {
                continue;
            }

            let sub = path.to_str().unwrap_or_default();
            match du_dir(
                sub,
                flag_h,
                flag_s,
                flag_a,
                flag_k,
                flag_H,
                flag_L,
                flag_x,
                current_root_dev,
                max_depth,
                cur_depth + 1,
                exit_code,
            ) {
                Ok(s) => total += s,
                Err(e) => {
                    eprintln!("du: {e}");
                    *exit_code = 1;
                }
            }
        }
    }

    if !flag_s || cur_depth == 0 {
        println!("{} {}", format_blocks(total, flag_h, flag_k), dir);
    }

    Ok(total)
}

/// Format a block count for display.
///
/// Returns the raw block count unless `flag_h` is true, in which case the
/// size is converted to bytes and rendered with a human-readable suffix.
fn format_blocks(blocks: u64, flag_h: bool, flag_k: bool) -> String {
    if flag_h {
        // Reconstruct bytes for human-readable formatting based on active block unit.
        let bytes = if flag_k { blocks * 1024 } else { blocks * 512 };
        human_size(bytes)
    } else {
        blocks.to_string()
    }
}

/// Convert a byte count to a compact human-readable string with an
/// appropriate unit suffix (`K`, `M`, `G`, …).
fn human_size(size: u64) -> String {
    const UNITS: &[&str] = &["", "K", "M", "G", "T", "P", "E"];
    let mut s = size as f64;
    let mut i = 0;

    while s >= 1024.0 && i < UNITS.len() - 1 {
        s /= 1024.0;
        i += 1;
    }

    if i == 0 {
        size.to_string()
    } else if s >= 10.0 {
        format!("{:.0}{}", s, UNITS[i])
    } else {
        format!("{:.1}{}", s, UNITS[i])
    }
}

register_command!(
    DU_CMD,
    "du",
    "hsad:HkLx",
    CommandFlags::BIN.bits(),
    du_main,
    description = "Estimate file space usage",
    help = "\
OPTIONS:
-h         Human-readable sizes (e.g. 1K, 234M, 2G).
-s         Display only a total for each argument (summary).
-a         Show counts for all files, not just directories.
-d DEPTH   Limit recursion to DEPTH levels below each argument.
-H         Follow symlinks specified on the command line.
-k         Write file sizes in 1024-byte blocks.
-L         Follow all symlinks.
-x         Limit to the same filesystem."
);
