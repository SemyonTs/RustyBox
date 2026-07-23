// =============================================================================
// registry — Command registration and lookup.
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Implementation inspired by Toybox (https://landley.net/toybox/)
// Toybox is copyrighted by Rob Landley, see NOTICE file for license details.
//
// Instead of the C macros `NEWTOY`/`OLDTOY`, the `linkme` crate is used to
// collect `CommandDef` instances into a distributed slice at link time.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use linkme::distributed_slice;

/// Distributed slice that collects every registered command definition.
#[distributed_slice]
pub static COMMANDS: [CommandDef] = [..];

/// Static descriptor for a single builtin command.
pub struct CommandDef {
    /// Command name as it appears on the command line.
    pub name: &'static str,
    /// Option string in Toybox `get_optflags` syntax.
    pub optstr: &'static str,
    /// Bitmask of `CommandFlags` values.
    pub flags: u32,
    /// Entry point: receives a mutable context and returns an exit code.
    pub run: fn(&mut Context) -> u8,
}

impl CommandDef {
    /// Return the flags field as a typed `CommandFlags` value.
    pub fn command_flags(&self) -> CommandFlags {
        CommandFlags::from_bits_truncate(self.flags)
    }
}

/// Look up a command by name using a binary search over the sorted slice.
///
/// Analogous to `toy_find()` in toybox/main.c:25.
pub fn find(name: &str) -> Option<&'static CommandDef> {
    let mut sorted: Vec<&CommandDef> = COMMANDS.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(b.name));
    sorted
        .binary_search_by(|c| c.name.cmp(name))
        .ok()
        .map(|i| sorted[i])
}

/// Return a sorted list of all registered commands.
pub fn all() -> Vec<&'static CommandDef> {
    let mut list: Vec<&CommandDef> = COMMANDS.iter().collect();
    list.sort_by(|a, b| a.name.cmp(b.name));
    list
}
