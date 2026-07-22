// =============================================================================
// link — Create a hard link.
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Implementation inspired by Toybox (https://landley.net/toybox/)
// Toybox is copyrighted by Rob Landley, see NOTICE file for license details.
//
// Usage: link FILE NEWLINK
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::fs;

/// Entry point for the `link` builtin.
///
/// The option string `"<2>2"` enforces exactly two positional arguments.
fn link_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "<2>2") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("link: {e}");
            return 1;
        }
    };
    let _ = opts;

    let file = &ctx.optargs[0];
    let newlink = &ctx.optargs[1];

    match fs::hard_link(file, newlink) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("link: cannot create link '{}' -> '{}': {}", newlink, file, e);
            1
        }
    }
}

register_command!(
    LINK_CMD,
    "link",
    "<2>2",
    CommandFlags::BIN.bits() | CommandFlags::USR.bits(),
    link_main
);