// =============================================================================
// unlink — Remove a single file.
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Implementation inspired by Toybox (https://landley.net/toybox/)
// Toybox is copyrighted by Rob Landley, see NOTICE file for license details.
//
// Usage: unlink FILE
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::fs;

/// Entry point for the `unlink` builtin.
///
/// The option string `"<1>1"` enforces exactly one positional argument.
fn unlink_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "<1>1") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("unlink: {e}");
            return 1;
        }
    };
    let _ = opts;

    let file = &ctx.optargs[0];
    match fs::remove_file(file) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("unlink: cannot remove '{}': {}", file, e);
            1
        }
    }
}

register_command!(
    UNLINK_CMD,
    "unlink",
    "<1>1",
    CommandFlags::BIN.bits() | CommandFlags::USR.bits(),
    unlink_main,
    description = "Remove a single file",
    help = "\
USAGE: 
unlink FILE"
);
