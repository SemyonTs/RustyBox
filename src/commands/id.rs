// =============================================================================
// id — Print user and group identity information.
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
//   -u   Print only the effective user ID.
//   -g   Print only the effective group ID.
//   -G   Print all group IDs.
//   -n   Print names instead of numeric IDs (combine with -u, -g, -G).
//   -r   Print real IDs instead of effective IDs.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;

/// Entry point for the `id` builtin.
///
/// Without options, a full human-readable summary is printed:
/// `uid=N(name) gid=N(name) groups=N(name),...`
fn id_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "ugnG(r)") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("id: {e}");
            return 1;
        }
    };

    let flag_u = opts.count('u') > 0;
    let flag_g = opts.count('g') > 0;
    let flag_n = opts.count('n') > 0;
    let flag_G = opts.count('G') > 0;

    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };

    // -u: print effective user ID (or name with -n).
    if flag_u {
        if flag_n {
            println!("{}", username(uid).unwrap_or_else(|| uid.to_string()));
        } else {
            println!("{}", uid);
        }
        return 0;
    }

    // -g: print effective group ID (or name with -n).
    if flag_g {
        if flag_n {
            println!("{}", groupname(gid).unwrap_or_else(|| gid.to_string()));
        } else {
            println!("{}", gid);
        }
        return 0;
    }

    // -G: print all supplementary group IDs.
    if flag_G {
        let groups = get_groups();
        if flag_n {
            let names: Vec<String> = groups
                .iter()
                .map(|g| groupname(*g).unwrap_or_else(|| g.to_string()))
                .collect();
            println!("{}", names.join(" "));
        } else {
            let ids: Vec<String> = groups.iter().map(|g| g.to_string()).collect();
            println!("{}", ids.join(" "));
        }
        return 0;
    }

    // Default: full identity summary.
    let uname = username(uid).unwrap_or_else(|| uid.to_string());
    let gname = groupname(gid).unwrap_or_else(|| gid.to_string());
    let groups = get_groups();
    let gnames: Vec<String> = groups
        .iter()
        .map(|g| format!("{}", groupname(*g).unwrap_or_else(|| g.to_string())))
        .collect();

    println!(
        "uid={}({}) gid={}({}) groups={}",
        uid, uname, gid, gname, gnames.join(",")
    );

    0
}

/// Look up a user name from a numeric UID via `getpwuid(3)`.
///
/// Returns `None` if the UID has no corresponding entry in the password
/// database.
fn username(uid: u32) -> Option<String> {
    unsafe {
        let pw = libc::getpwuid(uid);
        if pw.is_null() {
            None
        } else {
            let name = std::ffi::CStr::from_ptr((*pw).pw_name);
            Some(name.to_string_lossy().into_owned())
        }
    }
}

/// Look up a group name from a numeric GID via `getgrgid(3)`.
///
/// Returns `None` if the GID has no corresponding entry in the group
/// database.
fn groupname(gid: u32) -> Option<String> {
    unsafe {
        let gr = libc::getgrgid(gid);
        if gr.is_null() {
            None
        } else {
            let name = std::ffi::CStr::from_ptr((*gr).gr_name);
            Some(name.to_string_lossy().into_owned())
        }
    }
}

/// Retrieve the list of supplementary group IDs for the current process.
///
/// The effective GID is always included, even when `getgroups(2)` would
/// otherwise omit it.
fn get_groups() -> Vec<u32> {
    unsafe {
        // First call determines the required buffer size.
        let ngroups = libc::getgroups(0, std::ptr::null_mut());
        if ngroups <= 0 {
            return vec![libc::getgid()];
        }

        let mut groups: Vec<libc::gid_t> = Vec::with_capacity(ngroups as usize);
        groups.resize(ngroups as usize, 0);

        let count = libc::getgroups(ngroups, groups.as_mut_ptr());
        if count < 0 {
            vec![libc::getgid()]
        } else {
            groups[..count as usize].to_vec()
        }
    }
}

register_command!(
    ID_CMD,
    "id",
    "ugnG(r)",
    CommandFlags::BIN.bits(),
    id_main
);