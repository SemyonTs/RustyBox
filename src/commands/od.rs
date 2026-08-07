// =============================================================================
// od — Dump files in octal and other formats.
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
//   -A RADIX   Address radix: 'o' (octal), 'd' (decimal), 'x' (hex), 'n' (none)
//   -j BYTES   Skip BYTES bytes of input
//   -N BYTES   Limit dumping to BYTES bytes
//   -t TYPE    Output format type: 'a' (named chars), 'c' (ASCII chars),
//              'o' (octal), 'd' (signed decimal), 'x' (hex), 'u' (unsigned decimal)
//              TYPE may be followed by an optional size (e.g., o2, x4, dS, uL)
//              or by C,S,I,L for integer sizes, F,D,L for floating point.
//   -w BYTES   Width in bytes per line (default 16)
//   -v         Show all lines (do not abbreviate repeated lines with '*')
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::fs::File;
use std::io::{self, Read, Write};
use std::num::ParseIntError;

/// Specification for one output format.
#[derive(Debug, Clone, PartialEq)]
struct FormatSpec {
    /// Type character: 'a', 'c', 'o', 'd', 'u', 'x', 'f'
    ty: char,
    /// Number of bytes per element (1, 2, 4, 8, ...)
    size: usize,
    /// For floating point, the kind: F (float), D (double), L (long double)
    float_kind: Option<char>,
}

/// Entry point for the `od` builtin.
fn od_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "A:j:N:t:w:v") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("od: {e}");
            return 1;
        }
    };

    let flag_v = opts.count('v') > 0;

    // Address radix: default octal.
    let radix = opts.get_str('A').unwrap_or("o");
    let addr_base = match radix {
        "o" => 8,
        "d" => 10,
        "x" => 16,
        "n" => 0, // no address
        _ => {
            eprintln!("od: invalid address radix '{}'", radix);
            return 1;
        }
    };

    // Skip bytes (default 0).
    let skip_bytes = opts
        .get_str('j')
        .map(parse_size)
        .unwrap_or(Ok(0))
        .unwrap_or_else(|e| {
            eprintln!("od: invalid skip value: {}", e);
            std::process::exit(1);
        });

    // Limit bytes (default u64::MAX).
    let limit_bytes = opts
        .get_str('N')
        .map(parse_size)
        .unwrap_or(Ok(u64::MAX))
        .unwrap_or_else(|e| {
            eprintln!("od: invalid limit value: {}", e);
            std::process::exit(1);
        });

    // Width in bytes per line (default 16).
    let width = opts
        .get_str('w')
        .map(|s| s.parse::<usize>())
        .unwrap_or(Ok(16))
        .unwrap_or_else(|e| {
            eprintln!("od: invalid width: {}", e);
            std::process::exit(1);
        });

    // Parse format specifications from -t options.
    let mut format_specs: Vec<FormatSpec> = Vec::new();
    if opts.count('t') > 0 {
        for ts in opts.get_strs('t') {
            let mut specs = parse_type_string(ts);
            format_specs.append(&mut specs);
        }
    }

    // If no format specified, default to -t oS (octal short).
    if format_specs.is_empty() {
        format_specs.push(FormatSpec {
            ty: 'o',
            size: 2,
            float_kind: None,
        });
    }

    // Collect input sources: positional arguments, or stdin if none.
    let sources: Vec<&str> = if ctx.optargs.is_empty() {
        vec!["-"]
    } else {
        ctx.optargs.iter().map(|s| s.as_str()).collect()
    };

    let mut exit_code: u8 = 0;
    let mut first_file = true;

    for &source in &sources {
        let mut reader: Box<dyn Read> = if source == "-" {
            Box::new(io::stdin())
        } else {
            match File::open(source) {
                Ok(f) => Box::new(f),
                Err(e) => {
                    eprintln!("od: {}: {}", source, e);
                    exit_code = 1;
                    continue;
                }
            }
        };

        // Apply skip.
        if skip_bytes > 0 {
            let mut skipped = 0;
            let mut buf = [0u8; 8192];
            while skipped < skip_bytes {
                let to_read = std::cmp::min(skip_bytes - skipped, buf.len() as u64);
                let n = match reader.read(&mut buf[..to_read as usize]) {
                    Ok(n) => n,
                    Err(e) => {
                        eprintln!("od: error skipping: {}", e);
                        exit_code = 1;
                        break;
                    }
                };
                if n == 0 {
                    break;
                }
                skipped += n as u64;
            }
            if skipped < skip_bytes {
                // EOF before skipping enough.
                continue;
            }
        }

        // Read data with limit.
        let limit = if limit_bytes == u64::MAX {
            usize::MAX
        } else {
            limit_bytes as usize
        };
        let mut data = Vec::with_capacity(limit.min(1024 * 1024));
        let mut total = 0;
        let mut buf = [0u8; 8192];
        while total < limit {
            let to_read = std::cmp::min(limit - total, buf.len());
            let n = match reader.read(&mut buf[..to_read]) {
                Ok(n) => n,
                Err(e) => {
                    eprintln!("od: read error: {}", e);
                    exit_code = 1;
                    break;
                }
            };
            if n == 0 {
                break;
            }
            data.extend_from_slice(&buf[..n]);
            total += n;
        }

        if data.is_empty() {
            continue;
        }

        // Print a header for each file (except first).
        if !first_file && sources.len() > 1 {
            println!();
            println!("{}:", source);
        }
        first_file = false;

        dump_data(&data, addr_base, width, &format_specs, flag_v);
    }

    exit_code
}

/// Parse a size string with optional suffixes: b, k, M, G, etc.
fn parse_size(s: &str) -> Result<u64, ParseIntError> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(0);
    }
    let (num, mult) = if let Some(stripped) = s.strip_suffix('b') {
        (stripped, 512)
    } else if let Some(stripped) = s.strip_suffix('k') {
        (stripped, 1024)
    } else if let Some(stripped) = s.strip_suffix('M') {
        (stripped, 1024 * 1024)
    } else if let Some(stripped) = s.strip_suffix('G') {
        (stripped, 1024 * 1024 * 1024)
    } else if let Some(stripped) = s.strip_suffix('K') {
        (stripped, 1024)
    } else {
        (s, 1)
    };
    let n: u64 = num.parse()?;
    Ok(n * mult)
}

/// Parse a type string like "o2x4dS" into a list of FormatSpec.
fn parse_type_string(ts: &str) -> Vec<FormatSpec> {
    let mut specs = Vec::new();
    let chars: Vec<char> = ts.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ty = chars[i];
        i += 1;
        let mut size = None;
        let mut float_kind = None;

        // Parse optional size or suffix.
        if i < chars.len() {
            let c = chars[i];
            if c.is_ascii_digit() {
                // Parse number.
                let mut num = 0;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    num = num * 10 + (chars[i] as u32 - '0' as u32) as usize;
                    i += 1;
                }
                size = Some(num);
            } else if "CSILFDL".contains(c) {
                // Suffix for integer or float.
                match c {
                    'C' => size = Some(1),
                    'S' => size = Some(2),
                    'I' => size = Some(4),
                    'L' => size = Some(8),
                    'F' => {
                        size = Some(4);
                        float_kind = Some('F');
                    }
                    'D' => {
                        size = Some(8);
                        float_kind = Some('D');
                    }
                    'L' => {
                        size = Some(16);
                        float_kind = Some('L');
                    }
                    _ => {}
                }
                i += 1;
            }
        }

        // Default sizes.
        let size = size.unwrap_or_else(|| {
            match ty {
                'a' | 'c' => 1,
                'f' => 8, // double
                _ => 2,   // short for integer types
            }
        });

        specs.push(FormatSpec {
            ty,
            size,
            float_kind,
        });
    }
    specs
}

/// Dump the byte slice to stdout according to the given parameters.
fn dump_data(data: &[u8], addr_base: u8, width: usize, formats: &[FormatSpec], verbose: bool) {
    let total = data.len();
    if total == 0 {
        return;
    }

    let mut address = 0;
    let mut prev_block: Option<Vec<u8>> = None;
    let mut same_count = 0;

    let stdout = io::stdout();
    let mut out = stdout.lock();

    while address < total {
        let end = std::cmp::min(address + width, total);
        let block = &data[address..end];

        // Check for repeated block (if not verbose).
        if !verbose {
            if let Some(ref prev) = prev_block {
                if block == prev.as_slice() {
                    same_count += 1;
                    address = end;
                    continue;
                } else {
                    if same_count > 1 {
                        writeln!(out, "*").unwrap();
                    }
                    same_count = 1;
                }
            } else {
                same_count = 1;
            }
            prev_block = Some(block.to_vec());
        }

        // Print address.
        if addr_base != 0 {
            match addr_base {
                8 => write!(out, "{:07o} ", address).unwrap(),
                10 => write!(out, "{:07} ", address).unwrap(),
                16 => write!(out, "{:07x} ", address).unwrap(),
                _ => unreachable!(),
            }
        }

        // Print each format for this block.
        for (idx, spec) in formats.iter().enumerate() {
            if idx > 0 {
                // Separate multiple format columns with a tab.
                write!(out, "\t").unwrap();
            }
            dump_format(block, spec, &mut out);
        }

        writeln!(out).unwrap();

        address = end;
    }

    // If the last block was repeated and not printed, print "*".
    if !verbose && same_count > 1 {
        writeln!(out, "*").unwrap();
    }

    // Print final offset (after all data).
    if addr_base != 0 {
        match addr_base {
            8 => writeln!(out, "{:07o}", address).unwrap(),
            10 => writeln!(out, "{:07}", address).unwrap(),
            16 => writeln!(out, "{:07x}", address).unwrap(),
            _ => unreachable!(),
        }
    } else {
        // With -A n, the final offset line is optional; we emit an empty line.
        writeln!(out).unwrap();
    }
}

/// Dump a single block using the given format specification.
fn dump_format<W: Write>(block: &[u8], spec: &FormatSpec, out: &mut W) {
    let size = spec.size;
    let ty = spec.ty;

    // Special handling for 'a' and 'c' (character types) – they always output bytes.
    if ty == 'a' {
        dump_named_chars(block, out);
        return;
    }
    if ty == 'c' {
        dump_ascii_chars(block, out);
        return;
    }

    // For numeric/float types, process in chunks of `size` bytes.
    let mut i = 0;
    let len = block.len();
    while i < len {
        let chunk_end = std::cmp::min(i + size, len);
        let mut chunk = [0u8; 16]; // max size we support (16 for long double)
        let chunk_len = chunk_end - i;
        chunk[..chunk_len].copy_from_slice(&block[i..chunk_end]);
        // Pad with zeros if not enough bytes.
        if chunk_len < size {
            for j in chunk_len..size {
                chunk[j] = 0;
            }
        }

        // Convert according to type.
        match ty {
            'o' => {
                let val = match size {
                    1 => u8::from_ne_bytes(chunk[..1].try_into().unwrap()) as u64,
                    2 => u16::from_ne_bytes(chunk[..2].try_into().unwrap()) as u64,
                    4 => u32::from_ne_bytes(chunk[..4].try_into().unwrap()) as u64,
                    8 => u64::from_ne_bytes(chunk[..8].try_into().unwrap()),
                    _ => 0,
                };
                write!(out, " {:0width$o}", val, width = size * 3).unwrap();
            }
            'u' => {
                let val = match size {
                    1 => u8::from_ne_bytes(chunk[..1].try_into().unwrap()) as u64,
                    2 => u16::from_ne_bytes(chunk[..2].try_into().unwrap()) as u64,
                    4 => u32::from_ne_bytes(chunk[..4].try_into().unwrap()) as u64,
                    8 => u64::from_ne_bytes(chunk[..8].try_into().unwrap()),
                    _ => 0,
                };
                write!(out, " {:width$}", val, width = size * 3).unwrap();
            }
            'd' => {
                let val = match size {
                    1 => i8::from_ne_bytes(chunk[..1].try_into().unwrap()) as i64,
                    2 => i16::from_ne_bytes(chunk[..2].try_into().unwrap()) as i64,
                    4 => i32::from_ne_bytes(chunk[..4].try_into().unwrap()) as i64,
                    8 => i64::from_ne_bytes(chunk[..8].try_into().unwrap()),
                    _ => 0,
                };
                write!(out, " {:width$}", val, width = size * 3 + 1).unwrap();
            }
            'x' => {
                let val = match size {
                    1 => u8::from_ne_bytes(chunk[..1].try_into().unwrap()) as u64,
                    2 => u16::from_ne_bytes(chunk[..2].try_into().unwrap()) as u64,
                    4 => u32::from_ne_bytes(chunk[..4].try_into().unwrap()) as u64,
                    8 => u64::from_ne_bytes(chunk[..8].try_into().unwrap()),
                    _ => 0,
                };
                write!(out, " {:0width$x}", val, width = size * 2).unwrap();
            }
            'f' => {
                // Floating point: use native endianness.
                match size {
                    4 => {
                        let val = f32::from_ne_bytes(chunk[..4].try_into().unwrap());
                        write!(out, " {:e}", val).unwrap();
                    }
                    8 => {
                        let val = f64::from_ne_bytes(chunk[..8].try_into().unwrap());
                        write!(out, " {:e}", val).unwrap();
                    }
                    _ => {
                        // long double not supported; fallback to hex.
                        let val = u64::from_ne_bytes(chunk[..8].try_into().unwrap());
                        write!(out, " {:016x}", val).unwrap();
                    }
                }
            }
            _ => {}
        }

        i += size;
    }
}

/// Dump as named characters (like `od -t a`).
fn dump_named_chars<W: Write>(data: &[u8], out: &mut W) {
    for &b in data {
        let name = match b {
            0 => "nul",
            1 => "soh",
            2 => "stx",
            3 => "etx",
            4 => "eot",
            5 => "enq",
            6 => "ack",
            7 => "bel",
            8 => "bs",
            9 => "ht",
            10 => "nl", // or "lf"
            11 => "vt",
            12 => "ff",
            13 => "cr",
            14 => "so",
            15 => "si",
            16 => "dle",
            17 => "dc1",
            18 => "dc2",
            19 => "dc3",
            20 => "dc4",
            21 => "nak",
            22 => "syn",
            23 => "etb",
            24 => "can",
            25 => "em",
            26 => "sub",
            27 => "esc",
            28 => "fs",
            29 => "gs",
            30 => "rs",
            31 => "us",
            32 => "sp",
            127 => "del",
            _ if b >= 33 && b <= 126 => {
                // Printable ASCII
                write!(out, " {:3}", b as char).unwrap();
                continue;
            }
            _ => {
                // Non-ASCII: print as 3-digit octal
                write!(out, "{:04o}", b).unwrap();
                continue;
            }
        };
        write!(out, " {:>3}", name).unwrap();
    }
}

/// Dump as ASCII characters (like `od -t c`).
fn dump_ascii_chars<W: Write>(data: &[u8], out: &mut W) {
    for &b in data {
        let c = match b {
            0 => "\\0",
            7 => "\\a",
            8 => "\\b",
            9 => "\\t",
            10 => "\\n",
            11 => "\\v",
            12 => "\\f",
            13 => "\\r",
            27 => "\\e",
            92 => "\\\\",
            127 => "\\177",
            _ if b >= 32 && b <= 126 => {
                write!(out, " {:>3}", b as char).unwrap();
                continue;
            }
            _ => {
                // Non-printable: octal
                write!(out, "{:03o}", b).unwrap();
                continue;
            }
        };
        write!(out, " {:>3}", c).unwrap();
    }
}

// -----------------------------------------------------------------------------
// Registration.
// -----------------------------------------------------------------------------
register_command!(
    OD_CMD,
    "od",
    "A:j:N:t:w:v",
    CommandFlags::BIN.bits(),
    od_main,
    description = "Dump files in octal and other formats",
    help = "\
OPTIONS:
-A RADIX   Address radix: 'o' (octal), 'd' (decimal), 'x' (hex), 'n' (none)
-j BYTES   Skip BYTES bytes of input
-N BYTES   Limit dumping to BYTES bytes
-t TYPE    Output format type: 'a' (named chars), 'c' (ASCII chars),
           'o' (octal), 'd' (signed decimal), 'x' (hex), 'u' (unsigned decimal)
           TYPE may be followed by an optional size (e.g., o2, x4, dS, uL)
           or by C,S,I,L for integer sizes, F,D,L for floating point.
-w BYTES   Width in bytes per line (default 16)
-v         Show all lines (do not abbreviate repeated lines with '*')
"
);
