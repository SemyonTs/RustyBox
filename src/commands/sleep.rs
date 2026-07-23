// =============================================================================
// sleep — Delay for a specified amount of time.
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Implementation inspired by Toybox (https://landley.net/toybox/)
// Toybox is copyrighted by Rob Landley, see NOTICE file for license details.
//
// Accepts one or more duration arguments (fractional values permitted) with
// optional suffix: `s` (seconds, default), `m` (minutes), `h` (hours),
// `d` (days).  All durations are summed before sleeping.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::thread;
use std::time::Duration;

/// Entry point for the `sleep` builtin.
///
/// The option string `"<1"` enforces at least one positional argument.
fn sleep_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "<1") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("sleep: {e}");
            return 1;
        }
    };
    let _ = opts;

    // Sum all duration arguments.
    let mut total_nanos: u128 = 0;
    for arg in &ctx.optargs {
        match parse_duration(arg) {
            Ok(d) => total_nanos += d.as_nanos(),
            Err(e) => {
                eprintln!("sleep: {e}");
                return 1;
            }
        }
    }

    // Clamp to the maximum representable Duration and sleep.
    let dur = Duration::from_nanos(total_nanos.min(u64::MAX as u128) as u64);
    thread::sleep(dur);

    // On SIGINT the OS terminates the process; if we reach this point the
    // sleep completed uninterrupted.
    0
}

/// Parse a duration string with an optional fractional part and time suffix.
///
/// Supported suffixes:
///   `s` — seconds (default when absent)
///   `m` — minutes
///   `h` — hours
///   `d` — days
///
/// Arithmetic is saturating so that overflow does not cause a panic.
fn parse_duration(arg: &str) -> Result<Duration, String> {
    let bytes = arg.as_bytes();
    if bytes.is_empty() || (!bytes[0].is_ascii_digit() && bytes[0] != b'.') {
        return Err(format!("not a number: '{}'", arg));
    }

    let mut i = 0;

    // Integer part.
    let mut secs: u128 = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        secs = secs
            .saturating_mul(10)
            .saturating_add((bytes[i] - b'0') as u128);
        i += 1;
    }

    // Fractional part.
    let mut nanos: u128 = 0;
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        let mut scale: u128 = 1_000_000_000;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            scale /= 10;
            nanos += (bytes[i] - b'0') as u128 * scale;
            i += 1;
        }
    }

    // Time-unit suffix.
    if i < bytes.len() {
        let mult = match bytes[i] {
            b's' => 1u128,
            b'm' => 60,
            b'h' => 3600,
            b'd' => 86400,
            _ => return Err(format!("unknown suffix: '{}'", &arg[i..])),
        };
        i += 1;

        if i != bytes.len() {
            return Err(format!("unknown suffix: '{}'", &arg[i - 1..]));
        }

        secs = secs.saturating_mul(mult);
        nanos = nanos.saturating_mul(mult);
    }

    let total_nanos = secs.saturating_mul(1_000_000_000).saturating_add(nanos);

    Ok(Duration::from_nanos(
        total_nanos.min(u64::MAX as u128) as u64
    ))
}

register_command!(
    SLEEP_CMD,
    "sleep",
    "<1",
    CommandFlags::BIN.bits(),
    sleep_main
);
