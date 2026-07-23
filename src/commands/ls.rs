// =============================================================================
// ls — List directory contents.
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
//   -1      One entry per line (single-column output).
//   -a      Include entries whose names start with `.`.
//   -A      Like -a, but exclude `.` and `..`.
//   -d      List directories themselves, not their contents.
//   -F      Append a type indicator (`/`, `*`, `@`, `|`, `=`) to entries.
//   -h      Human-readable sizes (e.g. 1K, 234M).
//   -i      Print the inode number of each file.
//   -k      Use 1024-byte blocks (implied by default block size).
//   -l      Long format: permissions, link count, owner, group, size, time, name.
//   -R      Recursively list subdirectories.
//   -r      Reverse sort order.
//   -S      Sort by file size (largest first).
//   -t      Sort by modification time (newest first).
//   -u      With -t: sort by access time instead of modification time.
//   -C      Multi-column output (default when stdout is a terminal).
//   -x      Sort entries horizontally across columns.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::fs;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// In-memory representation of a single directory entry.
struct Entry {
    /// Base name of the entry.
    name: String,
    /// Full path to the entry.
    path: PathBuf,
    /// Cached filesystem metadata.
    meta: fs::Metadata,
    /// Whether this entry is a directory.
    is_dir: bool,
    /// For symlinks: the raw target path, if readable.
    symlink_target: Option<String>,
}

/// Entry point for the `ls` builtin.
///
/// When no arguments are given the current directory is listed.
fn ls_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "1aAdfhiklRrstu[-Cx1]") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("ls: {e}");
            return 1;
        }
    };

    let flag_l = opts.count('l') > 0;
    let flag_a = opts.count('a') > 0;
    let flag_A = opts.count('A') > 0;
    let flag_R = opts.count('R') > 0;
    let flag_d = opts.count('d') > 0;
    let flag_h = opts.count('h') > 0;
    let flag_i = opts.count('i') > 0;
    let flag_F = opts.count('F') > 0;
    let flag_1 = opts.count('1') > 0;
    let flag_r = opts.count('r') > 0;
    let flag_S = opts.count('S') > 0;
    let flag_t = opts.count('t') > 0;
    let flag_u = opts.count('u') > 0;

    let mut args: Vec<String> = ctx.optargs.clone();
    if args.is_empty() {
        args.push(".".to_string());
    }

    let mut exit_code: u8 = 0;
    let multiple = args.len() > 1;

    for (idx, arg) in args.iter().enumerate() {
        let meta = match fs::symlink_metadata(arg) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("ls: cannot access '{}': {}", arg, e);
                exit_code = 1;
                continue;
            }
        };

        // Non-directory arguments (or -d) are printed as single entries.
        if !meta.is_dir() || flag_d {
            let entries = vec![Entry {
                name: arg.clone(),
                path: PathBuf::from(arg),
                meta: meta.clone(),
                is_dir: false,
                symlink_target: None,
            }];

            if multiple {
                if idx > 0 {
                    println!();
                }
                println!("{arg}:");
            }

            print_entries(&entries, flag_l, flag_h, flag_i, flag_F, flag_1);
            continue;
        }

        // Directory listing with optional header when multiple arguments
        // are given.
        if multiple {
            if idx > 0 {
                println!();
            }
            println!("{arg}:");
        }

        list_dir(
            arg,
            flag_l,
            flag_a,
            flag_A,
            flag_R,
            flag_h,
            flag_i,
            flag_F,
            flag_1,
            flag_r,
            flag_S,
            flag_t,
            flag_u,
            &mut exit_code,
        );
    }

    exit_code
}

/// Read and display the contents of a single directory.
///
/// When `flag_R` is set, subdirectories are recursively visited.
fn list_dir(
    dir: &str,
    flag_l: bool,
    flag_a: bool,
    flag_A: bool,
    flag_R: bool,
    flag_h: bool,
    flag_i: bool,
    flag_F: bool,
    flag_1: bool,
    flag_r: bool,
    flag_S: bool,
    flag_t: bool,
    flag_u: bool,
    exit_code: &mut u8,
) {
    let mut entries: Vec<Entry> = match fs::read_dir(dir) {
        Ok(rd) => {
            let mut v = Vec::new();
            for item in rd {
                match item {
                    Ok(entry) => {
                        let name = entry.file_name().to_string_lossy().into_owned();
                        let path = entry.path();
                        match fs::symlink_metadata(&path) {
                            Ok(meta) => {
                                let symlink_target = if meta.file_type().is_symlink() {
                                    fs::read_link(&path)
                                        .ok()
                                        .map(|p| p.to_string_lossy().into_owned())
                                } else {
                                    None
                                };
                                v.push(Entry {
                                    name,
                                    path,
                                    meta: meta.clone(),
                                    is_dir: meta.is_dir(),
                                    symlink_target,
                                });
                            }
                            Err(_) => {}
                        }
                    }
                    Err(_) => {}
                }
            }
            v
        }
        Err(e) => {
            eprintln!("ls: cannot open directory '{}': {}", dir, e);
            *exit_code = 1;
            return;
        }
    };

    // Filter hidden entries according to -a / -A.
    entries.retain(|e| {
        if flag_a {
            true
        } else if flag_A {
            e.name != "." && e.name != ".."
        } else {
            !e.name.starts_with('.')
        }
    });

    sort_entries(&mut entries, flag_r, flag_S, flag_t, flag_u);

    print_entries(&entries, flag_l, flag_h, flag_i, flag_F, flag_1);

    // Recursive descent into subdirectories.
    if flag_R {
        for e in &entries {
            if e.is_dir && e.name != "." && e.name != ".." {
                let sub = if dir == "." {
                    e.name.clone()
                } else {
                    Path::new(dir).join(&e.name).to_string_lossy().into_owned()
                };
                println!();
                println!("{}:", sub);
                list_dir(
                    &sub, flag_l, flag_a, flag_A, flag_R, flag_h, flag_i, flag_F, flag_1, flag_r,
                    flag_S, flag_t, flag_u, exit_code,
                );
            }
        }
    }
}

/// Sort entries according to the active flags.
///
/// Precedence: `-S` (size) > `-t` (time) > name.  `-r` reverses the final
/// comparison.
fn sort_entries(entries: &mut [Entry], flag_r: bool, flag_S: bool, flag_t: bool, flag_u: bool) {
    entries.sort_by(|a, b| {
        let mut ord = std::cmp::Ordering::Equal;

        if flag_S {
            ord = b.meta.size().cmp(&a.meta.size());
        }

        if ord == std::cmp::Ordering::Equal && flag_t {
            let ta = if flag_u {
                a.meta.atime()
            } else {
                a.meta.mtime()
            };
            let tb = if flag_u {
                b.meta.atime()
            } else {
                b.meta.mtime()
            };
            ord = tb.cmp(&ta);
        }

        if ord == std::cmp::Ordering::Equal {
            ord = a.name.cmp(&b.name);
        }

        if flag_r {
            ord = ord.reverse();
        }

        ord
    });
}

/// Render a slice of entries to stdout.
fn print_entries(
    entries: &[Entry],
    flag_l: bool,
    flag_h: bool,
    flag_i: bool,
    flag_F: bool,
    flag_1: bool,
) {
    if flag_l {
        // Long format: emit a "total blocks" header first.
        let mut total_blocks = 0u64;
        for e in entries {
            total_blocks += e.meta.blocks() / 2; // 1024-byte → 512-byte blocks.
        }
        println!("total {} blocks", total_blocks);

        for e in entries {
            print_long(e, flag_h, flag_i, flag_F);
        }
    } else if flag_1 || entries.len() <= 1 {
        for e in entries {
            println!("{}{}", e.name, suffix(e, flag_F));
        }
    } else {
        // Multi-column layout, targeting an 80-column terminal.
        let width = entries.iter().map(|e| e.name.len() + 1).max().unwrap_or(1);
        let cols = std::cmp::max(1, 80 / width);

        for (i, e) in entries.iter().enumerate() {
            let s = format!("{}{}", e.name, suffix(e, flag_F));
            if i % cols == cols - 1 {
                println!("{}", s);
            } else {
                print!("{:<width$}", s, width = width);
            }
        }

        if entries.len() % cols != 0 {
            println!();
        }
    }
}

/// Return the `-F` type-indicator suffix for an entry.
fn suffix(e: &Entry, flag_F: bool) -> String {
    if !flag_F {
        return String::new();
    }

    let mode = e.meta.mode();

    if e.meta.is_dir() {
        "/".to_string()
    } else if e.meta.file_type().is_symlink() {
        "@".to_string()
    } else if e.meta.file_type().is_fifo() {
        "|".to_string()
    } else if e.meta.file_type().is_socket() {
        "=".to_string()
    } else if mode & 0o111 != 0 {
        "*".to_string()
    } else {
        String::new()
    }
}

/// Print a single entry in long (`-l`) format.
fn print_long(e: &Entry, flag_h: bool, flag_i: bool, flag_F: bool) {
    let mode = e.meta.mode();
    let perms = mode_string(mode);
    let nlink = e.meta.nlink();
    let uid = e.meta.uid();
    let gid = e.meta.gid();
    let owner = username(uid).unwrap_or_else(|| uid.to_string());
    let group = groupname(gid).unwrap_or_else(|| gid.to_string());
    let size = e.meta.size();
    let size_str = if flag_h {
        human_size(size)
    } else {
        size.to_string()
    };
    let mtime = e.meta.mtime();
    let date = format_time(mtime);

    let inode = if flag_i {
        format!("{} ", e.meta.ino())
    } else {
        String::new()
    };

    let name = if e.meta.file_type().is_symlink() {
        if let Some(t) = &e.symlink_target {
            format!("{} -> {}", e.name, t)
        } else {
            e.name.clone()
        }
    } else {
        format!("{}{}", e.name, suffix(e, flag_F))
    };

    println!(
        "{}{} {:>3} {:<8} {:<8} {:>8} {} {}",
        inode, perms, nlink, owner, group, size_str, date, name
    );
}

/// Build a 10-character `ls -l`-style mode string from a `st_mode` value.
///
/// Uses `as libc::mode_t` cast to work on both Linux (where mode_t is u32)
/// and FreeBSD (where mode_t is u16).
fn mode_string(mode: u32) -> String {
    let mut s = String::with_capacity(10);

    // File type.
    s.push(match (mode & 0o170000) as libc::mode_t {
        libc::S_IFDIR => 'd',
        libc::S_IFLNK => 'l',
        libc::S_IFCHR => 'c',
        libc::S_IFBLK => 'b',
        libc::S_IFIFO => 'p',
        libc::S_IFSOCK => 's',
        _ => '-',
    });

    // User.
    s.push(if mode & 0o400 != 0 { 'r' } else { '-' });
    s.push(if mode & 0o200 != 0 { 'w' } else { '-' });
    s.push(if mode & 0o100 != 0 {
        if mode & 0o4000 != 0 { 's' } else { 'x' }
    } else {
        if mode & 0o4000 != 0 { 'S' } else { '-' }
    });

    // Group.
    s.push(if mode & 0o040 != 0 { 'r' } else { '-' });
    s.push(if mode & 0o020 != 0 { 'w' } else { '-' });
    s.push(if mode & 0o010 != 0 {
        if mode & 0o2000 != 0 { 's' } else { 'x' }
    } else {
        if mode & 0o2000 != 0 { 'S' } else { '-' }
    });

    // Other.
    s.push(if mode & 0o004 != 0 { 'r' } else { '-' });
    s.push(if mode & 0o002 != 0 { 'w' } else { '-' });
    s.push(if mode & 0o001 != 0 {
        if mode & 0o1000 != 0 { 't' } else { 'x' }
    } else {
        if mode & 0o1000 != 0 { 'T' } else { '-' }
    });

    s
}

/// Format a byte count for human-readable display (`-h`).
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

/// Convert a Unix timestamp (seconds since epoch) into a human-readable
/// date string (`YYYY-MM-DD HH:MM`).
fn format_time(secs: i64) -> String {
    use std::time::{Duration, UNIX_EPOCH};
    let t = UNIX_EPOCH + Duration::from_secs(secs as u64);
    let dt: chrono::DateTime<chrono::Local> = t.into();
    dt.format("%Y-%m-%d %H:%M").to_string()
}

/// Look up a user name from a numeric UID via `getpwuid(3)`.
fn username(uid: u32) -> Option<String> {
    unsafe {
        let pw = libc::getpwuid(uid);
        if pw.is_null() {
            None
        } else {
            let name = std::ffi::CStr::from_ptr((*pw).pw_name);
            Some(name.to_string_lossy().into_owned())
        }
    }
}

/// Look up a group name from a numeric GID via `getgrgid(3)`.
fn groupname(gid: u32) -> Option<String> {
    unsafe {
        let gr = libc::getgrgid(gid);
        if gr.is_null() {
            None
        } else {
            let name = std::ffi::CStr::from_ptr((*gr).gr_name);
            Some(name.to_string_lossy().into_owned())
        }
    }
}

register_command!(
    LS_CMD,
    "ls",
    "1aAdfhiklRrstu[-Cx1]",
    CommandFlags::BIN.bits(),
    ls_main
);
