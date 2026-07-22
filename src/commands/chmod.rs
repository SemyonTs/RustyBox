// =============================================================================
// chmod — Change the file mode bits of each given file according to mode.
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
//   -R      Recursively change modes of directories and their contents.
//   -v      Output a diagnostic for every file processed.
//   -c      Like -v, but report only when a change is actually made.
//   -f      Suppress most error messages.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;

/// Entry point for the `chmod` builtin.
///
/// Parses the option string `Rcvf` and expects at least two positional
/// arguments: a mode specification followed by one or more file paths.
fn chmod_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "Rcvf") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("chmod: {e}");
            return 1;
        }
    };

    let flag_R = opts.count('R') > 0;
    let flag_v = opts.count('v') > 0;
    let flag_c = opts.count('c') > 0;
    let flag_f = opts.count('f') > 0;

    let mut args: Vec<String> = ctx.optargs.clone();
    if args.len() < 2 {
        eprintln!("chmod: not enough arguments");
        return 1;
    }

    // The first argument is the mode string; the remainder are file operands.
    let mode_str = args.remove(0);
    let mode = match parse_mode(&mode_str) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("chmod: invalid mode '{mode_str}': {e}");
            return 1;
        }
    };

    let mut exit_code: u8 = 0;
    for file in &args {
        if let Err(e) = chmod_one(file, &mode, flag_R, flag_v, flag_c, flag_f) {
            if !flag_f {
                eprintln!("chmod: {e}");
            }
            exit_code = 1;
        }
    }

    exit_code
}

/// Apply a mode change to a single filesystem entry, recursing into
/// directories when `flag_R` is set.
///
/// Symlinks are not followed: the mode of the link itself is never changed,
/// and when `-R` is active symlinks encountered during directory traversal
/// are skipped to avoid following them.
fn chmod_one(
    file: &str,
    mode: &ModeSpec,
    flag_R: bool,
    flag_v: bool,
    flag_c: bool,
    _flag_f: bool,
) -> Result<(), String> {
    let meta = fs::symlink_metadata(file).map_err(|e| format!("'{file}': {e}"))?;
    let old = meta.mode() & 0o7777;

    let new_mode = match mode {
        ModeSpec::Octal(m) => *m & 0o7777,
        ModeSpec::Symbolic(stanzas) => {
            let mut m = old;
            for st in stanzas {
                m = apply_stanza(m, st, meta.is_dir());
            }
            m & 0o7777
        }
    };

    // Report the change if -v is given, or if -c is given and the mode differs.
    if flag_v || (flag_c && old != new_mode) {
        eprintln!(
            "mode of '{}' changed from {:04o} ({}) to {:04o} ({})",
            file,
            old,
            mode_to_string(old),
            new_mode,
            mode_to_string(new_mode)
        );
    }

    fs::set_permissions(file, fs::Permissions::from_mode(new_mode))
        .map_err(|e| format!("'{file}': {e}"))?;

    // Recurse into subdirectories when -R is set.
    if flag_R && meta.is_dir() {
        let rd = fs::read_dir(file).map_err(|e| format!("'{file}': {e}"))?;
        for item in rd {
            let entry = item.map_err(|e| e.to_string())?;
            let name = entry.file_name().to_string_lossy().into_owned();

            // Skip the self-referential directory entries.
            if name == "." || name == ".." {
                continue;
            }

            let path = entry.path();
            let meta2 =
                fs::symlink_metadata(&path).map_err(|e| format!("'{path:?}': {e}"))?;

            // Do not descend into symlinks.
            if meta2.file_type().is_symlink() {
                continue;
            }

            chmod_one(
                &path.to_string_lossy(),
                mode,
                true, // recursion is always enabled for children
                flag_v,
                flag_c,
                _flag_f,
            )?;
        }
    }

    Ok(())
}

/// Parsed representation of a mode argument — either an octal mask or a
/// sequence of symbolic stanzas (e.g. `u+x,go-w`).
#[derive(Clone)]
enum ModeSpec {
    Octal(u32),
    Symbolic(Vec<Stanza>),
}

/// A single clause of a symbolic mode string.
///
/// Example: `u+x` → `who = 1 (user)`, `op = '+'`, `perm = 0o1 (execute)`.
#[derive(Clone)]
struct Stanza {
    /// Bitmask of affected classes: 1 = user, 2 = group, 4 = other.
    who: u32,
    /// Operator: `+`, `-`, or `=`.
    op: char,
    /// Permission bits to add, remove, or set (`r`, `w`, `x`, …).
    perm: u32,
    /// Special mode bits: `s` (setuid/setgid), `t` (sticky).
    special: u32,
}

/// Parse a mode string that is either an octal number or a comma-separated
/// list of symbolic stanzas.
fn parse_mode(s: &str) -> Result<ModeSpec, String> {
    // Octal mode: all characters must be valid octal digits.
    if s.chars().all(|c| c.is_digit(8)) && !s.is_empty() {
        let v = u32::from_str_radix(s, 8).map_err(|_| "octal number".to_string())?;
        return Ok(ModeSpec::Octal(v));
    }

    // Symbolic mode: split on commas, each part is a stanza.
    let mut stanzas = Vec::new();
    for part in s.split(',') {
        if part.is_empty() {
            continue;
        }
        let st = parse_stanza(part)?;
        stanzas.push(st);
    }

    if stanzas.is_empty() {
        return Err("empty mode".to_string());
    }

    Ok(ModeSpec::Symbolic(stanzas))
}

/// Parse a single symbolic stanza (e.g. `ug+x`, `o-w`, `a=rX`).
fn parse_stanza(s: &str) -> Result<Stanza, String> {
    let bytes: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut who = 0u32;

    // Collect who-specifier characters before the operator.
    while i < bytes.len() {
        match bytes[i] {
            'u' => who |= 1,
            'g' => who |= 2,
            'o' => who |= 4,
            'a' => who |= 7,
            '+' | '-' | '=' => break,
            _ => return Err(format!("unexpected character '{}'", bytes[i])),
        }
        i += 1;
    }

    // Default who to "all" if no explicit class was given.
    if who == 0 {
        who = 7;
    }

    if i >= bytes.len() {
        return Err("expected operator".to_string());
    }

    let op = bytes[i];
    i += 1;

    let mut perm = 0u32;
    let mut special = 0u32;

    // Collect permission letters after the operator.
    while i < bytes.len() {
        match bytes[i] {
            'r' => perm |= 0o4,
            'w' => perm |= 0o2,
            'x' => perm |= 0o1,
            's' => special |= 0o6000,
            't' => special |= 0o1000,
            'u' => perm |= 0o700, // copy user bits
            'g' => perm |= 0o070, // copy group bits
            'o' => perm |= 0o007, // copy other bits
            'X' => perm |= 0o111, // conditional execute (simplified: always sets x)
            _ => return Err(format!("unexpected character '{}'", bytes[i])),
        }
        i += 1;
    }

    Ok(Stanza {
        who,
        op,
        perm,
        special,
    })
}

/// Apply a single symbolic stanza to the current mode, returning the new mode.
fn apply_stanza(current: u32, st: &Stanza, _is_dir: bool) -> u32 {
    let mut m = current;

    // Handle special bits: setuid (04000), setgid (02000), sticky (01000).
    if st.special != 0 {
        let sval = st.special;
        for bit in 0..3 {
            if st.who & (1 << bit) != 0 {
                if bit == 0 {
                    m = match st.op {
                        '+' => m | 0o4000,
                        '-' => m & !0o4000,
                        '=' => (m & !0o4000) | (sval & 0o4000),
                        _ => m,
                    };
                } else if bit == 1 {
                    m = match st.op {
                        '+' => m | 0o2000,
                        '-' => m & !0o2000,
                        '=' => (m & !0o2000) | (sval & 0o2000),
                        _ => m,
                    };
                }
            }
        }
        // When who is "all" and operator is "=", replace all special bits.
        if st.who == 7 && st.op == '=' {
            m = (m & !0o7000) | (sval & 0o7000);
        }
    }

    // Handle standard permission bits (rwx) for user, group, and other.
    for bit in 0..3 {
        if st.who & (1 << bit) == 0 {
            continue;
        }
        let shift = (2 - bit) * 3;
        let mask = 0o7 << shift;
        let val = (st.perm & 0o7) << shift;
        m = match st.op {
            '+' => m | val,
            '-' => m & !mask,
            '=' => (m & !mask) | val,
            _ => m,
        };
    }

    m
}

/// Convert a numeric mode (lower 9 bits) into an `rwxrwxrwx` string.
fn mode_to_string(mode: u32) -> String {
    let mut s = String::with_capacity(9);
    s.push(if mode & 0o400 != 0 { 'r' } else { '-' });
    s.push(if mode & 0o200 != 0 { 'w' } else { '-' });
    s.push(if mode & 0o100 != 0 { 'x' } else { '-' });
    s.push(if mode & 0o040 != 0 { 'r' } else { '-' });
    s.push(if mode & 0o020 != 0 { 'w' } else { '-' });
    s.push(if mode & 0o010 != 0 { 'x' } else { '-' });
    s.push(if mode & 0o004 != 0 { 'r' } else { '-' });
    s.push(if mode & 0o002 != 0 { 'w' } else { '-' });
    s.push(if mode & 0o001 != 0 { 'x' } else { '-' });
    s
}

register_command!(
    CHMOD_CMD,
    "chmod",
    "Rcvf",
    CommandFlags::BIN.bits(),
    chmod_main
);