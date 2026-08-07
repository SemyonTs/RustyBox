// =============================================================================
// cksum — Print CRC checksum and byte count of each file.
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Implementation conforming to POSIX.1-2024 (cksum).
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::io::{self, Read};

/// Entry point for the `cksum` builtin.
///
/// Reads each file (or stdin if none) and prints:
///   CRC  SIZE  FILENAME
/// where SIZE is the number of bytes (as a decimal), and CRC is the
/// POSIX‑specified 32‑bit CRC (printed as an unsigned decimal).
fn cksum_main(ctx: &mut Context) -> u8 {
    // No options are defined for POSIX cksum.
    let _opts = match crate::args::parse(ctx, "") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("cksum: {e}");
            return 1;
        }
    };

    let files: Vec<&str> = if ctx.optargs.is_empty() {
        vec!["-"]
    } else {
        ctx.optargs.iter().map(|s| s.as_str()).collect()
    };

    let mut exit_code = 0;

    for &file in &files {
        let mut reader: Box<dyn Read> = if file == "-" {
            Box::new(io::stdin())
        } else {
            match std::fs::File::open(file) {
                Ok(f) => Box::new(f),
                Err(e) => {
                    eprintln!("cksum: {}: {}", file, e);
                    exit_code = 1;
                    continue;
                }
            }
        };

        match cksum_from_reader(&mut reader) {
            Ok((crc, size)) => {
                if file == "-" {
                    println!("{} {}", crc, size);
                } else {
                    println!("{} {} {}", crc, size, file);
                }
            }
            Err(e) => {
                eprintln!("cksum: {}: {}", file, e);
                exit_code = 1;
            }
        }
    }

    exit_code
}

/// Compute the POSIX cksum CRC‑32 and the byte count from any `Read` source.
///
/// The CRC is calculated over the file contents, then the file size (as a
/// 32‑bit little‑endian integer using the minimum number of bytes) is appended,
/// and finally the result is XORed with `0xFFFFFFFF`. The size returned is the
/// exact number of bytes read (as a `u64`).
fn cksum_from_reader<R: Read>(reader: &mut R) -> io::Result<(u32, u64)> {
    const POLY: u32 = 0x04C11DB7;
    let mut crc: u32 = 0;
    let mut size: u64 = 0;
    let mut buf = [0u8; 8192];

    // Process the file contents.
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        for &byte in &buf[..n] {
            crc ^= (byte as u32) << 24;
            for _ in 0..8 {
                if (crc & 0x8000_0000) != 0 {
                    crc = (crc << 1) ^ POLY;
                } else {
                    crc <<= 1;
                }
            }
        }
    }

    // Append the file size as little-endian, using the minimum number of bytes.
    let mut n = size as u32; // POSIX uses 32-bit size for the CRC calculation
    while n != 0 {
        let byte = (n & 0xFF) as u8;
        crc ^= (byte as u32) << 24;
        for _ in 0..8 {
            if (crc & 0x8000_0000) != 0 {
                crc = (crc << 1) ^ POLY;
            } else {
                crc <<= 1;
            }
        }
        n >>= 8;
    }

    // Final XOR.
    crc ^= 0xFFFF_FFFF;
    Ok((crc, size))
}

register_command!(
    CKSUM_CMD,
    "cksum",
    "",
    CommandFlags::BIN.bits(),
    cksum_main,
    description = "Print CRC checksum and byte count of each file",
    help = "\
Usage: cksum [FILE]...
   or: cksum -

Print the POSIX CRC-32 checksum and the byte count of each FILE.
With no FILE, or when FILE is -, read standard input."
);
