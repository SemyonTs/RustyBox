// =============================================================================
// flags — Command classification flags (analogous to toybox/lib/toyflags.h).
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Implementation inspired by Toybox (https://landley.net/toybox/)
// Toybox is copyrighted by Rob Landley, see NOTICE file for license details.
//
// Implemented as a thin `u32` wrapper so that flag values can be constructed
// in `const` contexts required by `linkme` distributed slices.
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CommandFlags(pub u32);

impl CommandFlags {
    pub const EMPTY: CommandFlags = CommandFlags(0);

    /// Command is located in the `bin` directory.
    pub const BIN: CommandFlags = CommandFlags(1 << 0);
    /// Command is located in the `sbin` directory.
    pub const SBIN: CommandFlags = CommandFlags(1 << 1);
    /// Do not fork before executing this command.
    pub const NOFORK: CommandFlags = CommandFlags(1 << 2);
    /// May fork if needed (e.g. for builtins that spawn subprocesses).
    pub const MAYFORK: CommandFlags = CommandFlags(1 << 3);
    /// Suppress automatic help output for this command.
    pub const NOHELP: CommandFlags = CommandFlags(1 << 4);
    /// Retain root privileges when running this command.
    pub const STAYROOT: CommandFlags = CommandFlags(1 << 5);
    /// This command requires root privileges.
    pub const NEEDROOT: CommandFlags = CommandFlags(1 << 6);
    /// Use line-buffered output.
    pub const LINEBUF: CommandFlags = CommandFlags(1 << 7);
    /// Command is a user utility (POSIX `usr` classification).
    pub const USR: CommandFlags = CommandFlags(1 << 8);

    /// Return the underlying `u32` bitmask.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Return `true` if all flags in `other` are set.
    pub const fn contains(self, other: CommandFlags) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for CommandFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        CommandFlags(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for CommandFlags {
    fn bitor_assign(&mut self, rhs: CommandFlags) {
        self.0 |= rhs.0;
    }
}
