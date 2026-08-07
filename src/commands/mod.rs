// =============================================================================
// Command module — builtin command registration and discovery.
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Implementation inspired by Toybox (https://landley.net/toybox/)
// Toybox is copyrighted by Rob Landley, see NOTICE file for license details.
//
// Each command module defines a static `CommandDef` via the
// `register_command!` macro.  The macro places the definition into the
// distributed slice `COMMANDS`, which is automatically collected at link
// time by the `linkme` crate.
// =============================================================================

/// Register a command so that it is discoverable at runtime.
///
/// `$flags` is a `u32` bitmask, typically built from `CommandFlags::X.bits()`.
///
/// ```ignore
/// register_command!(TRUE_CMD, "true", "", CommandFlags::NOHELP.bits(), true_main);
/// ```
#[macro_export]
macro_rules! register_command {
    ($static_name:ident, $name:expr, $optstr:expr, $flags:expr, $run:expr,
        description = $desc:expr, help = $help_text:expr $(,)?) => {
        $crate::__register_command_inner!(
            $static_name,
            $name,
            $optstr,
            $flags,
            $run,
            Some($desc),
            Some($help_text)
        );
    };

    ($static_name:ident, $name:expr, $optstr:expr, $flags:expr, $run:expr,
        description = $desc:expr $(,)?) => {
        $crate::__register_command_inner!(
            $static_name,
            $name,
            $optstr,
            $flags,
            $run,
            Some($desc),
            None
        );
    };

    ($static_name:ident, $name:expr, $optstr:expr, $flags:expr, $run:expr,
        help = $help_text:expr $(,)?) => {
        $crate::__register_command_inner!(
            $static_name,
            $name,
            $optstr,
            $flags,
            $run,
            None,
            Some($help_text)
        );
    };

    ($static_name:ident, $name:expr, $optstr:expr, $flags:expr, $run:expr $(,)?) => {
        $crate::__register_command_inner!($static_name, $name, $optstr, $flags, $run, None, None);
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __register_command_inner {
    ($static_name:ident, $name:expr, $optstr:expr, $flags:expr, $run:expr,
     $desc:expr, $help_text:expr) => {
        #[::linkme::distributed_slice($crate::registry::COMMANDS)]
        static $static_name: $crate::registry::CommandDef = $crate::registry::CommandDef {
            name: $name,
            optstr: $optstr,
            flags: $flags,
            run: $run,
            description: $desc,
            help: $help_text,
        };
    };
}

mod basename;
mod cal;
mod cat;
mod chmod;
mod chown;
mod cksum;
mod cp;
mod comm;
mod cut;
mod date;
mod df;
mod dirname;
mod du;
mod echo;
mod env;
mod expand;
mod false_;
mod grep;
mod head;
mod id;
mod kill;
mod link;
mod ln;
mod ls;
mod mkdir;
mod mv;
mod printf;
mod pwd;
mod readlink;
mod rm;
mod rmdir;
mod sed;
mod sleep;
mod sort;
mod split;
mod tail;
mod tee;
mod test;
mod tty;
mod touch;
mod tr;
mod true_;
mod od;
mod uname;
mod uniq;
mod unlink;
mod wc;
mod xargs;
