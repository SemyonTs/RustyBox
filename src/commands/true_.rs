// =============================================================================
// true — Return a zero exit code.
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Implementation inspired by Toybox (https://landley.net/toybox/)
// Toybox is copyrighted by Rob Landley, see NOTICE file for license details.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;

/// Entry point for the `true` builtin — always returns 0.
fn true_main(_ctx: &mut Context) -> u8 {
    0
}

register_command!(
    TRUE_CMD,
    "true",
    "",
    CommandFlags::BIN.bits() | CommandFlags::NOHELP.bits(),
    true_main
);
