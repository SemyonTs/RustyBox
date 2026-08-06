// =============================================================================
// tee — Read from stdin and write to stdout and files.
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
//   -a   Append to the given files rather than overwriting them.
//   -i   Ignore SIGINT (recognised, signal handling is process-wide).
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::fs::OpenOptions;
use std::io::{BufWriter, Read, Write};

/// Entry point for the `tee` builtin.
///
/// Data is read from stdin in 4 KiB chunks and written simultaneously to
/// stdout and every named output file.
fn tee_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "ai") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("tee: {e}");
            return 1;
        }
    };

    let flag_a = opts.count('a') > 0;

    // Open all output files, each wrapped in BufWriter for efficient writes.
    let mut handles: Vec<BufWriter<std::fs::File>> = Vec::with_capacity(ctx.optargs.len());
    for f in &ctx.optargs {
        match OpenOptions::new()
            .create(true)
            .write(true)
            .append(flag_a)
            .truncate(!flag_a)
            .open(f)
        {
            Ok(h) => handles.push(BufWriter::new(h)),
            Err(e) => {
                eprintln!("tee: '{}': {}", f, e);
                return 1;
            }
        }
    }

    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    let mut buf = [0u8; 4096];

    // Copy stdin to stdout and all output files.
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        out.write_all(&buf[..n]).ok();
        for h in &mut handles {
            h.write_all(&buf[..n]).ok();
        }
    }

    // Flush all buffered writers.
    out.flush().ok();
    for h in &mut handles {
        h.flush().ok();
    }

    0
}

register_command!(
    TEE_CMD,
    "tee",
    "ai",
    CommandFlags::BIN.bits(),
    tee_main,
    description = "Read from stdin and write to stdout and files",
    help = "\
OPTIONS:
-a   Append to the given files rather than overwriting them.
-i   Ignore SIGINT (recognised, signal handling is process-wide)."
);
