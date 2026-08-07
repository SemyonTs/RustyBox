// =============================================================================
// split — Split a file into pieces.
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Implementation inspired by Toybox (https://landley.net/toybox/)
// Toybox is copyrighted by Rob Landley, see NOTICE file for license details.
//
// POSIX-compliant split:
//   -b n[k|m]  split by bytes (with optional 1024/1048576 multiplier)
//   -l n       split by lines (default 1000)
//   -a n       suffix length (default 2)
//   -d         numeric suffixes (extension, not POSIX)
//   file       input file (or stdin)
//   name       output prefix (default "x")
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// Entry point for the `split` builtin.
fn split_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "l:b:a:d") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("split: {e}");
            return 1;
        }
    };

    let flag_d = opts.count('d') > 0;
    let suffix_len = opts.get_int('a').unwrap_or(2) as usize;

    // Parse -b (may have k/m suffix)
    let bytes_str = opts.get_str('b');
    let lines = opts.get_int('l');

    if bytes_str.is_some() && lines.is_some() {
        eprintln!("split: cannot specify both -b and -l");
        return 1;
    }

    // Determine split mode and size
    let (mode, size) = if let Some(bstr) = bytes_str {
        let (num, mult) = parse_size_suffix(bstr);
        if num == 0 {
            eprintln!("split: invalid byte count '{}'", bstr);
            return 1;
        }
        (Mode::Bytes, num * mult)
    } else {
        let n = lines.map(|l| l as usize).unwrap_or(1000);
        if n == 0 {
            eprintln!("split: invalid line count 0");
            return 1;
        }
        (Mode::Lines, n)
    };

    // Collect positional arguments: [FILE] [PREFIX]
    let mut args_iter = ctx.optargs.iter().map(|s| s.as_str());
    let file_arg = args_iter.next();
    let prefix = args_iter.next().unwrap_or("x");

    // Check name length: prefix + suffix (all 'z') must fit in NAME_MAX.
    let name_max = get_name_max();
    let max_name_len = prefix.len() + suffix_len;
    if max_name_len > name_max {
        eprintln!(
            "split: prefix '{}' plus suffix length {} exceeds NAME_MAX ({})",
            prefix, suffix_len, name_max
        );
        return 1;
    }

    // Open input
    let input: Box<dyn BufRead> = match file_arg {
        Some("-") | None => Box::new(BufReader::new(io::stdin())),
        Some(fname) => match File::open(fname) {
            Ok(f) => Box::new(BufReader::new(f)),
            Err(e) => {
                eprintln!("split: {}: {}", fname, e);
                return 1;
            }
        },
    };

    let mut suffix_gen = SuffixGenerator::new(suffix_len, flag_d);

    match mode {
        Mode::Lines => split_by_lines(input, prefix, size, &mut suffix_gen),
        Mode::Bytes => split_by_bytes(input, prefix, size, &mut suffix_gen),
    }
}

/// Mode of splitting.
#[derive(Copy, Clone, PartialEq)]
enum Mode {
    Lines,
    Bytes,
}

/// Parse a size string like "10k" or "2m" into (number, multiplier).
/// Returns (0, 1) on failure.
fn parse_size_suffix(s: &str) -> (usize, usize) {
    let s = s.trim();
    if s.is_empty() {
        return (0, 1);
    }
    let bytes = s.as_bytes();
    let last = bytes[bytes.len() - 1];
    let (num_str, mult) = match last {
        b'k' | b'K' => (&s[..s.len() - 1], 1024),
        b'm' | b'M' => (&s[..s.len() - 1], 1048576),
        _ => (s, 1),
    };
    let num = num_str.parse::<usize>().unwrap_or(0);
    (num, mult)
}

/// Return the system's NAME_MAX (maximum filename length) for the root
/// directory, or a fallback of 255 if not available.
fn get_name_max() -> usize {
    unsafe {
        let ret = libc::pathconf(b"/\0".as_ptr() as *const libc::c_char, libc::_PC_NAME_MAX);
        if ret == -1 { 255 } else { ret as usize }
    }
}

/// Generator of file name suffixes (alphabetic or numeric).
struct SuffixGenerator {
    /// Current index (0‑based).
    index: usize,
    /// Length of the suffix (number of characters/digits).
    len: usize,
    /// If true, use decimal digits; otherwise lowercase letters.
    numeric: bool,
    /// Maximum number of suffixes possible with current length.
    max_count: usize,
    /// Whether we have exhausted all suffixes.
    exhausted: bool,
}

impl SuffixGenerator {
    fn new(len: usize, numeric: bool) -> Self {
        let max_count = if numeric {
            10_usize.pow(len as u32)
        } else {
            26_usize.pow(len as u32)
        };
        SuffixGenerator {
            index: 0,
            len,
            numeric,
            max_count,
            exhausted: false,
        }
    }

    /// Generate the next suffix string. Returns `None` if the suffix space
    /// is exhausted (more pieces than available suffixes).
    /// Once exhausted, subsequent calls will keep returning `None`.
    fn next(&mut self) -> Option<String> {
        if self.exhausted {
            return None;
        }
        if self.index >= self.max_count {
            self.exhausted = true;
            return None;
        }
        let mut s = String::with_capacity(self.len);
        let mut n = self.index;
        for _ in 0..self.len {
            if self.numeric {
                let digit = (n % 10) as u8;
                s.push((b'0' + digit) as char);
                n /= 10;
            } else {
                let digit = (n % 26) as u8;
                s.push((b'a' + digit) as char);
                n /= 26;
            }
        }
        // Reverse to get correct order (least significant digit last)
        let reversed: String = s.chars().rev().collect();
        self.index += 1;
        Some(reversed)
    }

    /// Returns true if all possible suffixes have been used.
    fn is_exhausted(&self) -> bool {
        self.exhausted
    }
}

/// Split input by line count.
fn split_by_lines<R: BufRead>(
    mut reader: R,
    prefix: &str,
    lines_per_file: usize,
    suffix_gen: &mut SuffixGenerator,
) -> u8 {
    let mut line_count = 0;
    let mut output: Option<File> = None;
    let mut current_suffix = String::new();
    let mut error_occurred = false;

    let mut buf = String::new();
    loop {
        buf.clear();
        let n = match reader.read_line(&mut buf) {
            Ok(0) => break,
            Ok(_) => buf.len(),
            Err(e) => {
                eprintln!("split: read error: {}", e);
                return 1;
            }
        };
        if n == 0 {
            break;
        }

        // If we need a new file (first line or previous file full)
        if output.is_none() || line_count >= lines_per_file {
            if let Some(suffix) = suffix_gen.next() {
                current_suffix = suffix;
                let fname = format!("{}{}", prefix, current_suffix);
                match File::create(&fname) {
                    Ok(f) => output = Some(f),
                    Err(e) => {
                        eprintln!("split: cannot create '{}': {}", fname, e);
                        return 1;
                    }
                }
                line_count = 0;
            } else {
                // No more suffixes: we must keep writing to the last file.
                // This only happens if output is Some (we have at least one file).
                // If output is None, we cannot write anything → error.
                if output.is_none() {
                    eprintln!("split: cannot create first file (suffix space exhausted)");
                    return 1;
                }
                error_occurred = true;
                // Do not reset line_count; we continue writing to the same file.
            }
        }

        // Write the line to the current file (if it exists)
        if let Some(ref mut f) = output {
            if let Err(e) = f.write_all(buf.as_bytes()) {
                eprintln!("split: write error: {}", e);
                return 1;
            }
        } else {
            // No output file → this should not happen if we handled above.
            eprintln!("split: internal error: no output file");
            return 1;
        }
        line_count += 1;
    }

    if error_occurred { 1 } else { 0 }
}

/// Split input by byte count.
fn split_by_bytes<R: BufRead>(
    mut reader: R,
    prefix: &str,
    bytes_per_file: usize,
    suffix_gen: &mut SuffixGenerator,
) -> u8 {
    let mut byte_count = 0;
    let mut output: Option<File> = None;
    let mut current_suffix = String::new();
    let mut error_occurred = false;

    let mut buffer = [0u8; 8192];
    loop {
        let n = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                eprintln!("split: read error: {}", e);
                return 1;
            }
        };

        let mut pos = 0;
        while pos < n {
            // If we need a new file
            if output.is_none() || byte_count >= bytes_per_file {
                if let Some(suffix) = suffix_gen.next() {
                    current_suffix = suffix;
                    let fname = format!("{}{}", prefix, current_suffix);
                    match File::create(&fname) {
                        Ok(f) => output = Some(f),
                        Err(e) => {
                            eprintln!("split: cannot create '{}': {}", fname, e);
                            return 1;
                        }
                    }
                    byte_count = 0;
                } else {
                    // No more suffixes: keep writing to last file.
                    if output.is_none() {
                        eprintln!("split: cannot create first file (suffix space exhausted)");
                        return 1;
                    }
                    error_occurred = true;
                    // Do not reset byte_count; continue writing.
                }
            }

            // Write as many bytes as fit in the current file, or all remaining
            // if we are in exhausted state (then we write everything to the last file).
            let remaining_in_file = if suffix_gen.is_exhausted() {
                // When exhausted, we write the whole remaining chunk.
                n - pos
            } else {
                bytes_per_file - byte_count
            };
            let write_len = std::cmp::min(remaining_in_file, n - pos);
            if let Some(ref mut f) = output {
                if let Err(e) = f.write_all(&buffer[pos..pos + write_len]) {
                    eprintln!("split: write error: {}", e);
                    return 1;
                }
            } else {
                eprintln!("split: internal error: no output file");
                return 1;
            }
            pos += write_len;
            byte_count += write_len;
        }
    }

    if error_occurred { 1 } else { 0 }
}

register_command!(
    SPLIT_CMD,
    "split",
    "l:b:a:d",
    CommandFlags::BIN.bits(),
    split_main,
    description = "Split a file into pieces",
    help = "\
OPTIONS:
-b n[k|m] Split file into pieces of n bytes (with optional k=1024, m=1048576).
-l n      Split file into pieces of n lines (default 1000).
-a n      Use suffixes of length n (default 2).
-d        Use numeric suffixes instead of alphabetic (extension).

If FILE is omitted or is '-', read from stdin.
PREFIX defaults to 'x'."
);
