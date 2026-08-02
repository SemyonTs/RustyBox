// =============================================================================
// df — Report filesystem disk space usage.
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
//   -h      Human-readable sizes (e.g. 1K, 234M, 2G).
//   -a      Include dummy filesystems (recognised, not yet filtered).
//   -T      Print filesystem type column.
//   -k      Use 1024-byte units instead of the default 512-byte units.
//   -P      Produce output in the POSIX portable format.
//   -t      Include total allocated-space figures (always included in this implementation).
//   -i      Report inode usage instead of block usage.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::ffi::CString;
use std::fs;
use std::os::unix::fs::MetadataExt;

/// Entry point for the `df` builtin.
///
/// Reads mount information from `/proc/mounts` and queries each filesystem
/// via `statvfs(2)`.  When one or more path arguments are supplied only the
/// filesystems backing those paths are shown.
fn df_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "haTkPti") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("df: {e}");
            return 1;
        }
    };

    let flag_h = opts.count('h') > 0;
    let flag_T = opts.count('T') > 0;
    let flag_k = opts.count('k') > 0;
    let flag_P = opts.count('P') > 0;
    let flag_i = opts.count('i') > 0;
    let _flag_t = opts.count('t') > 0;

    let block_size: u64 = if flag_k { 1024 } else { 512 };

    // Obtain the list of currently mounted filesystems.
    let mounts = read_mounts();

    let mut exit_code = 0;
    let mut selected: Vec<Mount> = Vec::new();

    // Select which mounts to report.
    if ctx.optargs.is_empty() {
        selected = mounts.clone();
    } else {
        for arg in &ctx.optargs {
            match fs::metadata(arg) {
                Ok(meta) => {
                    let dev = meta.dev();
                    if let Some(m) = mounts.iter().find(|m| m.dev == dev) {
                        selected.push(m.clone());
                    } else {
                        // Fallback for paths not explicitly found in /proc/mounts
                        selected.push(Mount {
                            device: arg.clone(),
                            dev,
                            mount_point: arg.clone(),
                            fs_type: "unknown".to_string(),
                        });
                    }
                }
                Err(_) => {
                    eprintln!("df: {}: No such file or directory", arg);
                    exit_code = 1;
                }
            }
        }
    }

    // Print column headers.
    if flag_P {
        let blocks_str = if flag_k { "1024-blocks" } else { "512-blocks" };
        println!(
            "Filesystem {} Used Available Capacity Mounted on",
            blocks_str
        );
    } else if flag_i {
        if flag_T {
            println!(
                "{:<15} {:<10} {:<10} {:<10} {:<10} {:<5} {}",
                "Filesystem", "Type", "Inodes", "IUsed", "IFree", "IUse%", "Mounted"
            );
        } else {
            println!(
                "{:<15} {:<10} {:<10} {:<10} {:<5} {}",
                "Filesystem", "Inodes", "IUsed", "IFree", "IUse%", "Mounted"
            );
        }
    } else {
        if flag_T {
            println!(
                "{:<15} {:<10} {:<10} {:<10} {:<10} {:<5} {}",
                "Filesystem", "Type", "Size", "Used", "Avail", "Use%", "Mounted"
            );
        } else {
            println!(
                "{:<15} {:<10} {:<10} {:<10} {:<5} {}",
                "Filesystem", "Size", "Used", "Avail", "Use%", "Mounted"
            );
        }
    }

    // Emit one row per selected mount.
    for m in &selected {
        let mount_cstr = match CString::new(m.mount_point.as_str()) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let stat = unsafe {
            let mut buf: libc::statvfs = std::mem::zeroed();
            if libc::statvfs(mount_cstr.as_ptr(), &mut buf) != 0 {
                continue;
            }
            buf
        };

        let total_blocks = stat.f_blocks;
        let free_blocks = stat.f_bfree;
        let avail_blocks = stat.f_bavail;
        let used_blocks = if total_blocks > free_blocks {
            total_blocks - free_blocks
        } else {
            0
        };

        // POSIX percentage calculation: <space used> / (<space used> + <space free>).
        // Fractional results must be rounded to the next highest integer.
        let pct = if (used_blocks + avail_blocks) > 0 {
            ((used_blocks * 100) + (used_blocks + avail_blocks) - 1) / (used_blocks + avail_blocks)
        } else {
            0
        };

        if flag_P {
            // POSIX requires quantities expressed in blocks to be rounded up to the next higher unit.
            let total_units = (total_blocks * stat.f_frsize + block_size - 1) / block_size;
            let used_units = (used_blocks * stat.f_frsize + block_size - 1) / block_size;
            let avail_units = (avail_blocks * stat.f_frsize + block_size - 1) / block_size;

            println!(
                "{} {} {} {} {}% {}",
                m.device, total_units, used_units, avail_units, pct, m.mount_point
            );
        } else if flag_i {
            let total_i = stat.f_files;
            let free_i = stat.f_ffree;
            let used_i = if total_i > free_i {
                total_i - free_i
            } else {
                0
            };
            let avail_i = free_i;

            let pct_i = if (used_i + avail_i) > 0 {
                ((used_i * 100) + (used_i + avail_i) - 1) / (used_i + avail_i)
            } else {
                0
            };

            if flag_T {
                println!(
                    "{:<15} {:<10} {:<10} {:<10} {:<10} {:<4}% {}",
                    m.device, m.fs_type, total_i, used_i, avail_i, pct_i, m.mount_point
                );
            } else {
                println!(
                    "{:<15} {:<10} {:<10} {:<10} {:<4}% {}",
                    m.device, total_i, used_i, avail_i, pct_i, m.mount_point
                );
            }
        } else {
            let total_bytes = total_blocks * stat.f_frsize;
            let used_bytes = used_blocks * stat.f_frsize;
            let avail_bytes = avail_blocks * stat.f_frsize;

            if flag_T {
                println!(
                    "{:<15} {:<10} {:<10} {:<10} {:<10} {:<4}% {}",
                    m.device,
                    m.fs_type,
                    human(total_bytes, flag_h, block_size),
                    human(used_bytes, flag_h, block_size),
                    human(avail_bytes, flag_h, block_size),
                    pct,
                    m.mount_point
                );
            } else {
                println!(
                    "{:<15} {:<10} {:<10} {:<10} {:<4}% {}",
                    m.device,
                    human(total_bytes, flag_h, block_size),
                    human(used_bytes, flag_h, block_size),
                    human(avail_bytes, flag_h, block_size),
                    pct,
                    m.mount_point
                );
            }
        }
    }

    exit_code
}

/// A single entry from `/proc/mounts`.
#[derive(Clone)]
struct Mount {
    /// Device name (e.g. "/dev/sda1").
    device: String,
    /// Device identifier from `stat(2)`, used to match arguments to mounts.
    dev: u64,
    /// Mount point path.
    mount_point: String,
    /// Filesystem type string (e.g. "ext4", "tmpfs").
    fs_type: String,
}

/// Parse `/proc/mounts` and return a vector of `Mount` entries.
///
/// Each line is expected to contain at least three whitespace-separated
/// fields: device, mount point, and filesystem type.
fn read_mounts() -> Vec<Mount> {
    let mut result = Vec::new();
    if let Ok(content) = std::fs::read_to_string("/proc/mounts") {
        for line in content.lines() {
            let mut parts = line.split_whitespace();
            let device = match parts.next() {
                Some(d) => d.to_string(),
                None => continue,
            };
            let mount_point = match parts.next() {
                Some(mp) => mp.to_string(),
                None => continue,
            };
            let fs_type = match parts.next() {
                Some(ft) => ft.to_string(),
                None => continue,
            };
            let dev = if let Ok(meta) = fs::metadata(&mount_point) {
                meta.dev()
            } else {
                0
            };
            result.push(Mount {
                device,
                dev,
                mount_point,
                fs_type,
            });
        }
    }
    result
}

/// Format a byte count for display.
///
/// When `flag_h` is false the value is emitted in 512-byte units (the POSIX
/// default) or 1024-byte units if `block_size` is 1024. When true, a
/// human-readable suffix is appended (`K`, `M`, `G`, …) and one decimal
/// place is shown for values below 10 in the target unit.
fn human(bytes: u64, flag_h: bool, block_size: u64) -> String {
    if !flag_h {
        return (bytes / block_size).to_string();
    }

    const UNITS: &[&str] = &["", "K", "M", "G", "T", "P", "E"];
    let mut value = bytes as f64;
    let mut idx = 0;

    while value >= 1024.0 && idx < UNITS.len() - 1 {
        value /= 1024.0;
        idx += 1;
    }

    if idx == 0 {
        // Byte-level: display in block_size for consistency with the non-human path.
        (bytes / block_size).to_string()
    } else if value >= 10.0 {
        format!("{:.0}{}", value, UNITS[idx])
    } else {
        format!("{:.1}{}", value, UNITS[idx])
    }
}

register_command!(DF_CMD, "df", "haTkPti", CommandFlags::BIN.bits(), df_main);
