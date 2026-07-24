// =============================================================================
// uname — Print system information.
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
//   -a   Print all available information (equivalent to -snrvm).
//   -s   Print the kernel name.
//   -n   Print the network node hostname.
//   -r   Print the kernel release.
//   -v   Print the kernel version.
//   -m   Print the machine hardware name.
//   -o   Print the operating system name (recognised, not yet implemented).
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::mem::MaybeUninit;

/// Entry point for the `uname` builtin.
///
/// When no options are given `-s` is the default.
fn uname_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "asnrvmo") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("uname: {e}");
            return 1;
        }
    };

    let flag_a = opts.count('a') > 0;
    let flag_s = opts.count('s') > 0
        || flag_a
        || opts.count('a') == 0
            && opts.count('n') == 0
            && opts.count('r') == 0
            && opts.count('v') == 0
            && opts.count('m') == 0;
    let flag_n = opts.count('n') > 0 || flag_a;
    let flag_r = opts.count('r') > 0 || flag_a;
    let flag_v = opts.count('v') > 0 || flag_a;
    let flag_m = opts.count('m') > 0 || flag_a;

    // Get utsname once.
    let uts = unsafe {
        let mut uts: MaybeUninit<libc::utsname> = MaybeUninit::uninit();
        if libc::uname(uts.as_mut_ptr()) != 0 {
            eprintln!("uname: uname() failed");
            return 1;
        }
        uts.assume_init()
    };

    let sysname = cstr_from_bytes(&uts.sysname);
    let nodename = cstr_from_bytes(&uts.nodename);
    let release = cstr_from_bytes(&uts.release);
    let version = cstr_from_bytes(&uts.version);
    let machine = cstr_from_bytes(&uts.machine);

    // Build output string directly instead of Vec + join.
    let mut out = String::with_capacity(256);
    let mut first = true;

    let mut push_part = |part: &str| {
        if !first {
            out.push(' ');
        }
        out.push_str(part);
        first = false;
    };

    if flag_s {
        push_part(&sysname);
    }
    if flag_n {
        push_part(&nodename);
    }
    if flag_r {
        push_part(&release);
    }
    if flag_v {
        push_part(&version);
    }
    if flag_m {
        push_part(&machine);
    }

    println!("{out}");
    0
}

/// Convert a fixed-size `c_char` array to a Rust `String`,
/// stopping at the first NUL byte.
fn cstr_from_bytes<const N: usize>(bytes: &[libc::c_char; N]) -> String {
    let len = bytes.iter().position(|&c| c == 0).unwrap_or(N);
    let bytes_u8: &[u8] = unsafe { std::slice::from_raw_parts(bytes.as_ptr() as *const u8, len) };
    String::from_utf8_lossy(bytes_u8).into_owned()
}

register_command!(
    UNAME_CMD,
    "uname",
    "asnrvmo",
    CommandFlags::BIN.bits(),
    uname_main
);
