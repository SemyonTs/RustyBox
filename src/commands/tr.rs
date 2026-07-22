// =============================================================================
// tr — Translate or delete characters.
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
//   -d   Delete characters in SET1; do not translate.
//   -s   Squeeze consecutive repeats of characters in SET1 into one.
//   -c   Complement: operate on the set of characters not in SET1.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::io::{Read, Write};

fn tr_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "ds(c)") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("tr: {e}");
            return 1;
        }
    };

    let flag_d = opts.count('d') > 0;
    let flag_s = opts.count('s') > 0;
    let flag_c = opts.count('c') > 0;

    let args: Vec<String> = ctx.optargs.clone();
    if args.is_empty() {
        eprintln!("tr: missing SET1");
        return 1;
    }

    let set1_bytes = expand_set_bytes(&args[0]);
    let set2_bytes = if args.len() > 1 {
        expand_set_bytes(&args[1])
    } else {
        Vec::new()
    };

    // Build translation table for all 256 bytes.
    let mut table = [0u8; 256];
    let mut delete = [false; 256];       // fast delete flag
    let mut squeeze = [false; 256];      // fast squeeze set

    // Determine effective set1: complement if -c
    let effective_set1: Vec<u8> = if flag_c {
        (0u8..=255).filter(|b| !set1_bytes.contains(b)).collect()
    } else {
        set1_bytes
    };

    // Mark squeeze set before any modifications (for -s)
    if flag_s {
        for &b in &effective_set1 {
            squeeze[b as usize] = true;
        }
    }

    if flag_d {
        // Delete mode: mark bytes to delete
        for &b in &effective_set1 {
            delete[b as usize] = true;
        }
        // Table is identity for remaining bytes
        for i in 0..=255 {
            table[i] = i as u8;
        }
    } else {
        // Translate mode: build mapping
        // Default: identity
        for i in 0..=255 {
            table[i] = i as u8;
        }
        // Map set1 -> set2
        for (i, &b) in effective_set1.iter().enumerate() {
            let mapped = if i < set2_bytes.len() {
                set2_bytes[i]
            } else {
                *set2_bytes.last().unwrap_or(&b)
            };
            table[b as usize] = mapped;
        }
    }

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    let mut in_buf = [0u8; 4096];
    let mut out_buf = Vec::with_capacity(4096);

    loop {
        let n = match stdin.lock().read(&mut in_buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };

        out_buf.clear();
        let slice = &in_buf[..n];

        if flag_d {
            // Delete: just keep bytes not marked for deletion
            out_buf.extend(slice.iter().filter(|&&b| !delete[b as usize]));
        } else {
            // Translate: apply table
            for &b in slice {
                out_buf.push(table[b as usize]);
            }
        }

        // Apply squeeze if -s
        if flag_s {
            let mut squeezed = Vec::with_capacity(out_buf.len());
            let mut last = None;
            for &b in &out_buf {
                if squeeze[b as usize] {
                    if last == Some(b) {
                        continue; // skip consecutive duplicate of squeeze set
                    }
                }
                squeezed.push(b);
                last = Some(b);
            }
            stdout.write_all(&squeezed).ok();
        } else {
            stdout.write_all(&out_buf).ok();
        }
    }

    0
}

/// Expand a TR set string into a vector of bytes.
///
/// Supports backslash escapes and character ranges (e.g., `a-z`).
/// Operates on bytes, assuming single-byte characters for ranges.
fn expand_set_bytes(s: &str) -> Vec<u8> {
    let mut result = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            let b = match bytes[i + 1] {
                b'n' => b'\n',
                b't' => b'\t',
                b'r' => b'\r',
                b'0' => b'\0',
                b'\\' => b'\\',
                other => other,
            };
            result.push(b);
            i += 2;
        } else if bytes[i] == b'-' && i > 0 && i + 1 < bytes.len() {
            // Expand byte range: e.g., 'a-z'
            let start = bytes[i - 1];
            let end = bytes[i + 1];
            if start < end {
                for b in (start + 1)..=end {
                    result.push(b);
                }
            }
            i += 1;
        } else {
            result.push(bytes[i]);
            i += 1;
        }
    }

    result
}

register_command!(
    TR_CMD,
    "tr",
    "ds(c)",
    CommandFlags::BIN.bits(),
    tr_main
);