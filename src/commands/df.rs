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
    let opts = match crate::args::parse(ctx, "haT") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("df: {e}");
            return 1;
        }
    };

    let flag_h = opts.count('h') > 0;
    let flag_T = opts.count('T') > 0;

    // Obtain the list of currently mounted filesystems.
    let mounts = read_mounts();

    // Select which mounts to report — borrow from `mounts` instead of cloning.
    let mut selected: Vec<&Mount> = Vec::new();
    if ctx.optargs.is_empty() {
        selected = mounts.iter().collect();
    } else {
        for arg in &ctx.optargs {
            if let Ok(meta) = fs::metadata(arg) {
                let dev = meta.dev();
                for m in &mounts {
                    if m.dev == dev {
                        selected.push(m);
                        break;
                    }
                }
            }
        }
    }

    // Print column headers.
    if flag_T {
        println!(
            "{:<12} {:<15} {:<10} {:<10} {:<5} {}",
            "Filesystem", "Size", "Used", "Avail", "Use%", "Mounted"
        );
    } else {
        println!(
            "{:<15} {:<10} {:<10} {:<5} {}",
            "Size", "Used", "Avail", "Use%", "Mounted"
        );
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

        let total = stat.f_blocks * stat.f_frsize;
        let avail = stat.f_bavail * stat.f_frsize;
        let used = total - stat.f_bfree * stat.f_frsize;
        let pct = if total > 0 {
            (used * 100 / total) as u32
        } else {
            0
        };

        if flag_T {
            println!(
                "{:<12} {:<15} {:<10} {:<10} {:<4}% {}",
                m.fs_type,
                human(total, flag_h),
                human(used, flag_h),
                human(avail, flag_h),
                pct,
                m.mount_point
            );
        } else {
            println!(
                "{:<15} {:<10} {:<10} {:<4}% {}",
                human(total, flag_h),
                human(used, flag_h),
                human(avail, flag_h),
                pct,
                m.mount_point
            );
        }
    }

    0
}

/// A single entry from `/proc/mounts`.
struct Mount {
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
            // Parse only the fields we need without collecting into a Vec.
            let mount_point = match parts.nth(1) {
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
/// When `flag_h` is false the value is emitted as kilobytes (the historical
/// default for `df`).  When true, a human-readable suffix is appended (`K`,
/// `M`, `G`, …) and one decimal place is shown for values below 10 in the
/// target unit.
fn human(bytes: u64, flag_h: bool) -> String {
    if !flag_h {
        return (bytes / 1024).to_string();
    }

    const UNITS: &[&str] = &["", "K", "M", "G", "T", "P", "E"];
    let mut value = bytes as f64;
    let mut idx = 0;

    while value >= 1024.0 && idx < UNITS.len() - 1 {
        value /= 1024.0;
        idx += 1;
    }

    if idx == 0 {
        // Byte-level: display in kB for consistency with the non-human path.
        (bytes / 1024).to_string()
    } else if value >= 10.0 {
        format!("{:.0}{}", value, UNITS[idx])
    } else {
        format!("{:.1}{}", value, UNITS[idx])
    }
}

register_command!(DF_CMD, "df", "haT", CommandFlags::BIN.bits(), df_main);
