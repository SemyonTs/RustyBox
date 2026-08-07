// =============================================================================
// tty — Print the file name of the terminal connected to standard input.
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Supported options:
//   -s      Silent: do not print anything; only return the exit status.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::os::unix::io::AsRawFd;

/// Entry point for the `tty` builtin.
///
/// Exit codes:
///   0 — standard input is a terminal.
///   1 — standard input is not a terminal.
///   2 — an error occurred (e.g., ttyname failed).
fn tty_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "s") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("tty: {e}");
            return 2;
        }
    };

    let silent = opts.count('s') > 0;

    // Get the raw file descriptor for stdin.
    let fd = std::io::stdin().as_raw_fd();

    // Check if it's a terminal.
    let is_tty = unsafe { libc::isatty(fd) == 1 };

    if !is_tty {
        if !silent {
            println!("not a tty");
        }
        return 1;
    }

    // Retrieve the terminal name.
    let tty_name = unsafe {
        // ttyname_r is thread-safe and preferred.
        let mut buf = [0u8; 256];
        let ret = libc::ttyname_r(fd, buf.as_mut_ptr() as *mut libc::c_char, buf.len());
        if ret == 0 {
            // Convert the C string to a Rust string.
            let cstr = std::ffi::CStr::from_ptr(buf.as_ptr() as *const libc::c_char);
            Some(cstr.to_string_lossy().into_owned())
        } else {
            None
        }
    };

    match tty_name {
        Some(name) => {
            if !silent {
                println!("{}", name);
            }
            0
        }
        None => {
            // ttyname_r failed, but isatty said it's a terminal — unexpected.
            if !silent {
                eprintln!("tty: cannot get terminal name");
            }
            2
        }
    }
}

register_command!(
    TTY_CMD,
    "tty",
    "s",
    CommandFlags::BIN.bits(),
    tty_main,
    description = "Print the file name of the terminal connected to standard input",
    help = "\
OPTIONS:
-s      Silent: do not print anything; only return the exit status.

EXIT STATUS:
0   standard input is a terminal.
1   standard input is not a terminal.
2   an error occurred."
);
