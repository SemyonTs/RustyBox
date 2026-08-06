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
//   --color[=WHEN]  Colorize output (auto, always, never).
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::ffi::CStr;
use std::fs;
use std::io::{BufWriter, IsTerminal, Write};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::time::{Duration, UNIX_EPOCH};

/// In-memory representation of a single directory entry.
struct Entry {
    /// Base name of the entry.
    name: String,
    /// Full path to the entry.
    path: PathBuf,
    /// Cached filesystem metadata (only what's needed: mode, size, times, etc.).
    mode: u32,
    size: u64,
    blocks: u64,
    ino: u64,
    nlink: u64,
    uid: u32,
    gid: u32,
    atime: i64,
    mtime: i64,
    is_dir: bool,
    is_symlink: bool,
    is_fifo: bool,
    is_socket: bool,
    is_exec: bool,
    /// For symlinks: the raw target path, if readable.
    symlink_target: Option<String>,
}

/// Entry point for the `ls` builtin.
///
/// When no arguments are given the current directory is listed.
fn ls_main(ctx: &mut Context) -> u8 {
    // Pre-process argv to extract --color options before the short-option parser sees them.
    let mut color_mode = "never".to_string();
    let filtered_argv: Vec<String> = ctx
        .argv
        .iter()
        .enumerate()
        .filter_map(|(i, arg)| {
            if i == 0 {
                return Some(arg.clone()); // Keep the command name
            }
            match arg.as_str() {
                "--color" | "--color=auto" => {
                    color_mode = "auto".to_string();
                    None
                }
                "--color=always" | "--color=yes" => {
                    color_mode = "always".to_string();
                    None
                }
                "--color=never" | "--color=no" => {
                    color_mode = "never".to_string();
                    None
                }
                _ => Some(arg.clone()),
            }
        })
        .collect();

    // Replace ctx.argv so the standard parser ignores the long option
    ctx.argv = filtered_argv;

    let use_color = if color_mode == "always" {
        true
    } else if color_mode == "auto" {
        std::io::stdout().is_terminal()
    } else {
        false
    };

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

    let mut exit_code: u8 = 0;
    let multiple = ctx.optargs.len() > 1;

    let stdout = std::io::stdout();
    let mut writer = BufWriter::new(stdout.lock());

    // Reusable buffers for output formatting.
    let mut out_buf = String::with_capacity(4096);

    if ctx.optargs.is_empty() {
        list_dir(
            ".",
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
            use_color,
            &mut exit_code,
            multiple,
            false, // first
            &mut writer,
            &mut out_buf,
        );
    } else {
        let mut first = true;
        for arg in &ctx.optargs {
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
                let entry = entry_from_meta(arg, &meta);
                let entries = [entry];

                if multiple {
                    if !first {
                        writeln!(writer).ok();
                    }
                    writeln!(writer, "{arg}:").ok();
                }

                print_entries(
                    &entries,
                    flag_l,
                    flag_h,
                    flag_i,
                    flag_F,
                    flag_1,
                    use_color,
                    &mut writer,
                    &mut out_buf,
                );
            } else {
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
                    use_color,
                    &mut exit_code,
                    multiple,
                    first,
                    &mut writer,
                    &mut out_buf,
                );
            }
            first = false;
        }
    }

    writer.flush().ok();
    exit_code
}

/// Build an Entry from a path and its metadata (for non-directory or -d).
fn entry_from_meta(path: &str, meta: &fs::Metadata) -> Entry {
    let mode = meta.mode();
    let file_type = meta.file_type();
    Entry {
        name: path.to_string(),
        path: PathBuf::from(path),
        mode,
        size: meta.len(),
        blocks: meta.blocks(),
        ino: meta.ino(),
        nlink: meta.nlink(),
        uid: meta.uid(),
        gid: meta.gid(),
        atime: meta.atime(),
        mtime: meta.mtime(),
        is_dir: file_type.is_dir(),
        is_symlink: file_type.is_symlink(),
        is_fifo: file_type.is_fifo(),
        is_socket: file_type.is_socket(),
        is_exec: mode & 0o111 != 0,
        symlink_target: if file_type.is_symlink() {
            fs::read_link(path)
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
        } else {
            None
        },
    }
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
    use_color: bool,
    exit_code: &mut u8,
    multiple: bool,
    first: bool,
    writer: &mut BufWriter<std::io::StdoutLock>,
    out_buf: &mut String,
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
                                let mode = meta.mode();
                                let file_type = meta.file_type();
                                let symlink_target = if file_type.is_symlink() {
                                    fs::read_link(&path)
                                        .ok()
                                        .map(|p| p.to_string_lossy().into_owned())
                                } else {
                                    None
                                };
                                v.push(Entry {
                                    name,
                                    path,
                                    mode,
                                    size: meta.len(),
                                    blocks: meta.blocks(),
                                    ino: meta.ino(),
                                    nlink: meta.nlink(),
                                    uid: meta.uid(),
                                    gid: meta.gid(),
                                    atime: meta.atime(),
                                    mtime: meta.mtime(),
                                    is_dir: file_type.is_dir(),
                                    is_symlink: file_type.is_symlink(),
                                    is_fifo: file_type.is_fifo(),
                                    is_socket: file_type.is_socket(),
                                    is_exec: mode & 0o111 != 0,
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

    if multiple {
        if !first {
            writeln!(writer).ok();
        }
        writeln!(writer, "{dir}:").ok();
    }

    print_entries(
        &entries, flag_l, flag_h, flag_i, flag_F, flag_1, use_color, writer, out_buf,
    );

    // Recursive descent into subdirectories.
    if flag_R {
        for e in &entries {
            if e.is_dir && e.name != "." && e.name != ".." {
                let sub = if dir == "." {
                    e.name.clone()
                } else {
                    let mut p = String::with_capacity(dir.len() + 1 + e.name.len());
                    p.push_str(dir);
                    p.push('/');
                    p.push_str(&e.name);
                    p
                };
                writeln!(writer).ok();
                writeln!(writer, "{}:", sub).ok();
                list_dir(
                    &sub, flag_l, flag_a, flag_A, flag_R, flag_h, flag_i, flag_F, flag_1, flag_r,
                    flag_S, flag_t, flag_u, use_color, exit_code, false, false, writer, out_buf,
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
            ord = b.size.cmp(&a.size);
        }

        if ord == std::cmp::Ordering::Equal && flag_t {
            let ta = if flag_u { a.atime } else { a.mtime };
            let tb = if flag_u { b.atime } else { b.mtime };
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

/// Render a slice of entries to stdout via `writer`.
fn print_entries(
    entries: &[Entry],
    flag_l: bool,
    flag_h: bool,
    flag_i: bool,
    flag_F: bool,
    flag_1: bool,
    use_color: bool,
    writer: &mut BufWriter<std::io::StdoutLock>,
    out_buf: &mut String,
) {
    if flag_l {
        // Long format: emit a "total blocks" header first.
        let total_blocks: u64 = entries.iter().map(|e| e.blocks).sum::<u64>() / 2;
        writeln!(writer, "total {} blocks", total_blocks).ok();

        for e in entries {
            print_long(e, flag_h, flag_i, flag_F, use_color, writer, out_buf);
        }
    } else if flag_1 || entries.len() <= 1 {
        for e in entries {
            out_buf.clear();
            out_buf.push_str(&format_entry_name(e, flag_F, use_color));
            writeln!(writer, "{out_buf}").ok();
        }
    } else {
        // Multi-column layout, targeting an 80-column terminal.
        // Calculate width based on raw string length to avoid ANSI code interference.
        let width = entries
            .iter()
            .map(|e| e.name.len() + if flag_F { suffix(e).len() } else { 0 } + 1)
            .max()
            .unwrap_or(1);
        let cols = std::cmp::max(1, 80 / width);

        for (i, e) in entries.iter().enumerate() {
            let raw_len = e.name.len() + if flag_F { suffix(e).len() } else { 0 };
            let display_name = format_entry_name(e, flag_F, use_color);

            if i % cols == cols - 1 {
                writeln!(writer, "{}", display_name).ok();
            } else {
                // Pad with spaces after the colored string.
                // Since format_entry_name ends with a reset code, the padding will be uncolored.
                let padding = width.saturating_sub(raw_len);
                write!(writer, "{}{:width$}", display_name, "", width = padding).ok();
            }
        }

        if entries.len() % cols != 0 {
            writeln!(writer).ok();
        }
    }
}

/// Format the entry name with optional colorization and `-F` suffix.
fn format_entry_name(e: &Entry, flag_F: bool, use_color: bool) -> String {
    if !use_color {
        let mut s = e.name.clone();
        if flag_F {
            s.push_str(suffix(e));
        }
        return s;
    }

    let color = if e.is_dir {
        "\x1b[01;34m" // Bold Blue
    } else if e.is_symlink {
        "\x1b[01;36m" // Bold Cyan
    } else if e.is_exec {
        "\x1b[01;32m" // Bold Green
    } else if e.is_socket {
        "\x1b[01;35m" // Bold Magenta
    } else if e.is_fifo {
        "\x1b[33m" // Yellow
    } else {
        ""
    };

    let reset = "\x1b[0m";
    let mut s = String::new();
    if !color.is_empty() {
        s.push_str(color);
    }
    s.push_str(&e.name);
    if flag_F {
        s.push_str(suffix(e));
    }
    if !color.is_empty() {
        s.push_str(reset);
    }
    s
}

/// Return the `-F` type-indicator suffix for an entry as a `&str`.
fn suffix(e: &Entry) -> &'static str {
    if e.is_dir {
        "/"
    } else if e.is_symlink {
        "@"
    } else if e.is_fifo {
        "|"
    } else if e.is_socket {
        "="
    } else if e.is_exec {
        "*"
    } else {
        ""
    }
}

/// Print a single entry in long (`-l`) format.
fn print_long(
    e: &Entry,
    flag_h: bool,
    flag_i: bool,
    flag_F: bool,
    use_color: bool,
    writer: &mut BufWriter<std::io::StdoutLock>,
    out_buf: &mut String,
) {
    let perms = mode_string(e.mode);
    let owner = username(e.uid).unwrap_or_else(|| e.uid.to_string());
    let group = groupname(e.gid).unwrap_or_else(|| e.gid.to_string());
    let size_str = if flag_h {
        human_size(e.size)
    } else {
        e.size.to_string()
    };
    let date = format_time(e.mtime);

    out_buf.clear();
    if flag_i {
        use std::fmt::Write;
        write!(out_buf, "{} ", e.ino).unwrap();
    }
    use std::fmt::Write;
    write!(
        out_buf,
        "{} {:>3} {:<8} {:<8} {:>8} {} ",
        perms, e.nlink, owner, group, size_str, date
    )
    .unwrap();

    out_buf.push_str(&format_entry_name(e, flag_F, use_color));

    if e.is_symlink {
        if let Some(t) = &e.symlink_target {
            // Colorize the symlink arrow and target (cyan, matching standard ls behavior)
            out_buf.push_str(" -> \x1b[01;36m");
            out_buf.push_str(t);
            out_buf.push_str("\x1b[0m");
        }
    }

    writeln!(writer, "{out_buf}").ok();
}

/// Build a 10-character `ls -l`-style mode string from a `st_mode` value.
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
            let name = CStr::from_ptr((*pw).pw_name);
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
            let name = CStr::from_ptr((*gr).gr_name);
            Some(name.to_string_lossy().into_owned())
        }
    }
}

register_command!(
    LS_CMD,
    "ls",
    "1aAdfhiklRrstu[-Cx1]",
    CommandFlags::BIN.bits(),
    ls_main,
    description = "List directory contents",
    help = "\
OPTIONS:
-1      One entry per line (single-column output).
-a      Include entries whose names start with `.`.
-A      Like -a, but exclude `.` and `..`.
-d      List directories themselves, not their contents.
-F      Append a type indicator (`/`, `*`, `@`, `|`, `=`) to entries.
-h      Human-readable sizes (e.g. 1K, 234M).
-i      Print the inode number of each file.
-k      Use 1024-byte blocks (implied by default block size).
-l      Long format: permissions, link count, owner, group, size, time, name.
-R      Recursively list subdirectories.
-r      Reverse sort order.
-S      Sort by file size (largest first).
-t      Sort by modification time (newest first).
-u      With -t: sort by access time instead of modification time.
-C      Multi-column output (default when stdout is a terminal).
-x      Sort entries horizontally across columns.
--color[=WHEN]  Colorize output (auto, always, never)."
);
