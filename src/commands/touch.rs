// =============================================================================
// touch — Change file access and modification times.
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
//   -a        Change the access time only.
//   -c        Do not create any files that do not already exist.
//   -d DATE   Use DATE instead of the current time (YYYY-MM-DD hh:mm:ss).
//   -m        Change the modification time only.
//   -r FILE   Use the timestamps of FILE as the new values.
//   -t TIME   Use TIME in the format [[CC]YY]MMDDhhmm[.ss].
//   -h        Affect symlinks themselves rather than their targets.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use filetime::FileTime;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Entry point for the `touch` builtin.
///
/// The option string `"<1acd:fmr:t:h[!dtr]"` requires at least one file
/// operand and marks `-d`, `-t`, and `-r` as mutually exclusive.
fn touch_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "<1acd:fmr:t:h[!dtr]") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("touch: {e}");
            return 1;
        }
    };

    let flag_a = opts.count('a') > 0;
    let flag_m = opts.count('m') > 0;
    let flag_c = opts.count('c') > 0;
    let flag_h = opts.count('h') > 0;
    let date = opts.get_str('d').unwrap_or("");
    let time = opts.get_str('t').unwrap_or("");
    let ref_file = opts.get_str('r').unwrap_or("");

    // Determine the target timestamp(s).
    let (atime, mtime) = if !ref_file.is_empty() {
        match fs::symlink_metadata(ref_file) {
            Ok(m) => (m.atime(), m.mtime()),
            Err(e) => {
                eprintln!("touch: '{}': {}", ref_file, e);
                return 1;
            }
        }
    } else if !date.is_empty() {
        match parse_date(date) {
            Ok(t) => (t, t),
            Err(e) => {
                eprintln!("touch: invalid date '{}': {}", date, e);
                return 1;
            }
        }
    } else if !time.is_empty() {
        match parse_time(time) {
            Ok(t) => (t, t),
            Err(e) => {
                eprintln!("touch: invalid time '{}': {}", time, e);
                return 1;
            }
        }
    } else {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        (now, now)
    };

    // Decide which timestamps to modify based on -a / -m.
    let (atime_opt, mtime_opt): (Option<i64>, Option<i64>) = if flag_a && !flag_m {
        (Some(atime), None)
    } else if flag_m && !flag_a {
        (None, Some(mtime))
    } else {
        (Some(atime), Some(mtime))
    };

    let mut exit_code: u8 = 0;
    for file in &ctx.optargs {
        if !set_times(file, atime_opt, mtime_opt, flag_c, flag_h) {
            exit_code = 1;
        }
    }

    exit_code
}

/// Apply the requested timestamps to a single file.
///
/// When the file does not exist and `-c` is absent a zero-length file is
/// created first.  Returns `true` on success.
fn set_times(
    file: &str,
    atime: Option<i64>,
    mtime: Option<i64>,
    no_create: bool,
    no_deref: bool,
) -> bool {
    let set_result = if no_deref {
        set_symlink_times(file, atime, mtime)
    } else {
        set_file_times(file, atime, mtime)
    };

    if set_result.is_ok() {
        return true;
    }

    // If the file is missing and creation is allowed, create it and retry.
    if !no_create {
        if fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(file)
            .is_ok()
        {
            let retry = if no_deref {
                set_symlink_times(file, atime, mtime)
            } else {
                set_file_times(file, atime, mtime)
            };
            return retry.is_ok();
        }
    }

    eprintln!("touch: cannot set times on '{}'", file);
    false
}

/// Convert a Unix timestamp (seconds) to `SystemTime`.
fn to_system_time(secs: i64) -> SystemTime {
    if secs >= 0 {
        UNIX_EPOCH + Duration::from_secs(secs as u64)
    } else {
        UNIX_EPOCH - Duration::from_secs((-secs) as u64)
    }
}

/// Set access and modification times on a regular file or directory.
fn set_file_times(file: &str, atime: Option<i64>, mtime: Option<i64>) -> std::io::Result<()> {
    let a: FileTime = to_system_time(atime.unwrap_or_else(now_secs)).into();
    let m: FileTime = to_system_time(mtime.unwrap_or_else(now_secs)).into();
    filetime::set_file_times(file, a, m)
}

/// Set access and modification times on a symlink itself (`-h`).
fn set_symlink_times(file: &str, atime: Option<i64>, mtime: Option<i64>) -> std::io::Result<()> {
    let a: FileTime = to_system_time(atime.unwrap_or_else(now_secs)).into();
    let m: FileTime = to_system_time(mtime.unwrap_or_else(now_secs)).into();
    filetime::set_symlink_file_times(file, a, m)
}

/// Return the current time as seconds since the Unix epoch.
fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// Parse a `-d` date string: `YYYY-MM-DD hh:mm:ss` (or with `T` separator).
fn parse_date(s: &str) -> Result<i64, String> {
    let s = s.replace('T', " ");
    let parts: Vec<&str> = s.split(' ').collect();

    if parts.len() != 2 {
        return Err("expected 'YYYY-MM-DD hh:mm:ss'".to_string());
    }

    let date: Vec<&str> = parts[0].split('-').collect();
    let time: Vec<&str> = parts[1].split(':').collect();

    if date.len() != 3 || time.len() < 2 {
        return Err("invalid date format".to_string());
    }

    let y: i64 = date[0].parse().map_err(|_| "year".to_string())?;
    let mo: i64 = date[1].parse().map_err(|_| "month".to_string())?;
    let d: i64 = date[2].parse().map_err(|_| "day".to_string())?;
    let h: i64 = time[0].parse().map_err(|_| "hours".to_string())?;
    let mi: i64 = time[1].parse().map_err(|_| "minutes".to_string())?;
    let sec: i64 = if time.len() > 2 {
        time[2].parse().unwrap_or(0)
    } else {
        0
    };

    unix_from_ymdhms(y, mo, d, h, mi, sec)
}

/// Parse a `-t` time string: `[[CC]YY]MMDDhhmm[.ss]`.
fn parse_time(s: &str) -> Result<i64, String> {
    let (main, _frac) = if let Some(i) = s.find('.') {
        (&s[..i], &s[i + 1..])
    } else {
        (s, "")
    };

    let digits: Vec<char> = main.chars().collect();

    let (y, mo, d, h, mi, sec) = match digits.len() {
        8 => (
            1970,
            to_num(&digits[0..2])?,
            to_num(&digits[2..4])?,
            to_num(&digits[4..6])?,
            to_num(&digits[6..8])?,
            0,
        ),
        10 => (
            to_num(&digits[0..2])? + 2000,
            to_num(&digits[2..4])?,
            to_num(&digits[4..6])?,
            to_num(&digits[6..8])?,
            to_num(&digits[8..10])?,
            0,
        ),
        12 => (
            to_num(&digits[0..4])?,
            to_num(&digits[4..6])?,
            to_num(&digits[6..8])?,
            to_num(&digits[8..10])?,
            to_num(&digits[10..12])?,
            0,
        ),
        _ => return Err("expected [[CC]YY]MMDDhhmm".to_string()),
    };

    unix_from_ymdhms(y, mo, d, h, mi, sec)
}

/// Parse a slice of `char` into an `i64`.
fn to_num(d: &[char]) -> Result<i64, String> {
    d.iter()
        .collect::<String>()
        .parse::<i64>()
        .map_err(|_| "not a number".to_string())
}

/// Convert year, month, day, hour, minute, second to seconds since the Unix
/// epoch (proleptic Gregorian, 1970 onward).
fn unix_from_ymdhms(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64) -> Result<i64, String> {
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return Err("invalid date".to_string());
    }

    let mut days = 0i64;

    // Count complete years since 1970.
    for yy in 1970..y {
        days += if is_leap(yy) { 366 } else { 365 };
    }

    // Count complete months in the current year.
    let month_days = [0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for mm in 1..mo {
        days += month_days[mm as usize];
        if mm == 2 && is_leap(y) {
            days += 1;
        }
    }

    days += d - 1;

    Ok(days * 86400 + h * 3600 + mi * 60 + s)
}

/// Return `true` when `y` is a leap year in the Gregorian calendar.
fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

register_command!(
    TOUCH_CMD,
    "touch",
    "<1acd:fmr:t:h[!dtr]",
    CommandFlags::BIN.bits(),
    touch_main
);
