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
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::fs;
use std::os::unix::fs::MetadataExt;

/// Entry point for the `du` builtin.
///
/// When no paths are given the current directory is used by default.
fn du_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "hsad:") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("du: {e}");
            return 1;
        }
    };

    let flag_h = opts.count('h') > 0;
    let flag_s = opts.count('s') > 0;
    let flag_a = opts.count('a') > 0;
    let depth = opts.get_int('d').unwrap_or(-1);

    let args: Vec<String> = ctx.optargs.clone();
    let dirs: Vec<String> = if args.is_empty() {
        vec![".".to_string()]
    } else {
        args
    };

    let mut exit_code: u8 = 0;
    for dir in &dirs {
        match du_dir(dir, flag_h, flag_s, flag_a, depth, 0, &mut exit_code) {
            Ok(_) => {}
            Err(e) => {
                eprintln!("du: {e}");
                exit_code = 1;
            }
        }
    }

    exit_code
}

/// Recursively accumulate disk usage for a single filesystem entry.
///
/// Returns the total number of 512-byte blocks allocated to the subtree
/// rooted at `dir`.  The size of each node is reported according to the
/// active flags (summary, all-files, depth limit).
fn du_dir(
    dir: &str,
    flag_h: bool,
    flag_s: bool,
    flag_a: bool,
    max_depth: i64,
    cur_depth: i64,
    exit_code: &mut u8,
) -> Result<u64, String> {
    let meta = fs::symlink_metadata(dir).map_err(|e| format!("'{dir}': {e}"))?;

    // Non-directory entry: report its own block count when -a is active.
    if !meta.is_dir() {
        let size = meta.blocks() / 2; // Convert 1024-byte blocks to 512-byte units.
        if flag_a {
            println!("{} {}", format_blocks(size, flag_h), dir);
        }
        return Ok(size);
    }

    // Directory: start with the blocks consumed by the directory itself.
    let mut total = meta.blocks() / 2;

    // Descend into children unless summarising (-s) or the depth limit has
    // been reached.
    if !flag_s && (max_depth < 0 || cur_depth < max_depth) {
        let rd = fs::read_dir(dir).map_err(|e| format!("'{dir}': {e}"))?;
        for item in rd {
            let entry = item.map_err(|e| e.to_string())?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();

            // Skip the self-referential directory entries.
            if name == "." || name == ".." {
                continue;
            }

            let meta2 = match fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(_) => continue,
            };

            // Do not follow symlinks; they contribute zero blocks to the
            // parent's total.
            if meta2.file_type().is_symlink() {
                continue;
            }

            let sub = path.to_string_lossy().into_owned();
            match du_dir(
                &sub,
                flag_h,
                flag_s,
                flag_a,
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

    // Report the aggregate total for this level.
    // When -s is active only the top-level argument is printed.
    if !flag_s || cur_depth == 0 {
        println!("{} {}", format_blocks(total, flag_h), dir);
    }

    Ok(total)
}

/// Format a block count for display.
///
/// Returns the raw block count unless `flag_h` is true, in which case the
/// size is converted to bytes and rendered with a human-readable suffix.
fn format_blocks(blocks: u64, flag_h: bool) -> String {
    let kb = blocks * 1024; // 512-byte blocks → bytes.
    if flag_h {
        human_size(kb)
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
    "hsad:",
    CommandFlags::BIN.bits(),
    du_main
);