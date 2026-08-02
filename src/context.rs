// =============================================================================
// context — Global command execution context.
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Implementation inspired by Toybox (https://landley.net/toybox/)
// Toybox is copyrighted by Rob Landley, see NOTICE file for license details.
// =============================================================================

use crate::registry::CommandDef;

/// Execution context for a single command invocation.
///
/// Holds the argument vector, parsed option flags, remaining positional
/// arguments, and the exit code to be returned to the caller.
pub struct Context {
    /// Descriptor of the currently executing command.
    pub which: &'static CommandDef,
    /// Full argument vector, including `argv[0]`.
    pub argv: Vec<String>,
    /// Bitmask of recognised options (bit *n* corresponds to the *n*-th
    /// option in the option string, rightmost = bit 0).
    pub optflags: u64,
    /// Positional arguments remaining after option parsing.
    /// Reused across commands via `clear()` to avoid reallocation.
    pub optargs: Vec<String>,
    /// Exit code to be returned to the shell.
    pub exitval: u8,
}

impl Context {
    /// Create a new context for the given command and argument vector.
    /// Pre-allocates capacity for `optargs` to minimise reallocations
    /// during argument parsing.
    pub fn new(which: &'static CommandDef, argv: Vec<String>) -> Self {
        let cap = argv.len();
        Context {
            which,
            argv,
            optflags: 0,
            optargs: Vec::with_capacity(cap),
            exitval: 0,
        }
    }

    /// Return `true` if the option at the given bit index is set.
    /// Uses `wrapping_shl` to avoid undefined behaviour for out-of-range bits.
    pub fn has_opt(&self, bit: u32) -> bool {
        self.optflags & (1u64.wrapping_shl(bit)) != 0
    }

    /// Reset the context for another command invocation.
    /// The `optargs` buffer is reused by clearing it, preserving capacity.
    pub fn clear(&mut self, new_argv: Vec<String>) {
        self.argv = new_argv;
        self.optflags = 0;
        self.optargs.clear();
        self.exitval = 0;
    }
}
