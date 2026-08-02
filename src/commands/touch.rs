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
    let time_spec = opts.get_str('t').unwrap_or("");
    let ref_file = opts.get_str('r').unwrap_or("");

    // Determine the target timestamp(s).
    let (target_atime, target_mtime) = if !ref_file.is_empty() {
        match fs::symlink_metadata(ref_file) {
            Ok(m) => (
                FileTime::from_unix_time(m.atime(), m.atime_nsec() as u32),
                FileTime::from_unix_time(m.mtime(), m.mtime_nsec() as u32),
            ),
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
    } else if !time_spec.is_empty() {
        match parse_time(time_spec) {
            Ok(t) => (t, t),
            Err(e) => {
                eprintln!("touch: invalid time '{}': {}", time_spec, e);
                return 1;
            }
        }
    } else {
        let now = FileTime::from_system_time(SystemTime::now());
        (now, now)
    };

    // Decide which timestamps to modify based on -a / -m.
    // Per POSIX: if neither -a nor -m is specified, behave as if both were.
    let update_atime = flag_a || (!flag_a && !flag_m);
    let update_mtime = flag_m || (!flag_a && !flag_m);

    let mut exit_code: u8 = 0;
    for file in &ctx.optargs {
        if !set_times(
            file,
            update_atime,
            update_mtime,
            target_atime,
            target_mtime,
            flag_c,
            flag_h,
        ) {
            exit_code = 1;
        }
    }

    exit_code
}

/// Apply the requested timestamps to a single file.
///
/// When the file does not exist and `-c` is absent a zero-length file is
/// created first. Returns `true` on success.
fn set_times(
    file: &str,
    update_atime: bool,
    update_mtime: bool,
    target_atime: FileTime,
    target_mtime: FileTime,
    no_create: bool,
    no_deref: bool,
) -> bool {
    // Check existence first for -c handling
    let exists = if no_deref {
        fs::symlink_metadata(file).is_ok()
    } else {
        fs::metadata(file).is_ok()
    };

    if !exists {
        if no_create {
            // POSIX: -c means silently skip non-existent files
            return true;
        }
        // Create the file
        if let Err(e) = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(file)
        {
            eprintln!("touch: cannot touch '{}': {}", file, e);
            return false;
        }
    }

    // Read existing metadata to preserve timestamps when only one is updated
    let meta = if no_deref {
        fs::symlink_metadata(file)
    } else {
        fs::metadata(file)
    };

    let meta = match meta {
        Ok(m) => m,
        Err(e) => {
            eprintln!("touch: cannot stat '{}': {}", file, e);
            return false;
        }
    };

    let final_atime = if update_atime {
        target_atime
    } else {
        FileTime::from_unix_time(meta.atime(), meta.atime_nsec() as u32)
    };

    let final_mtime = if update_mtime {
        target_mtime
    } else {
        FileTime::from_unix_time(meta.mtime(), meta.mtime_nsec() as u32)
    };

    let result = if no_deref {
        filetime::set_symlink_file_times(file, final_atime, final_mtime)
    } else {
        filetime::set_file_times(file, final_atime, final_mtime)
    };

    if let Err(e) = result {
        eprintln!("touch: cannot set times on '{}': {}", file, e);
        return false;
    }

    true
}

/// Parse a `-d` date string: `YYYY-MM-DD hh:mm:ss` (or with `T` separator).
fn parse_date(s: &str) -> Result<FileTime, String> {
    let (date_part, time_part) = if let Some((d, t)) = s.split_once('T') {
        (d, t)
    } else if let Some((d, t)) = s.split_once(' ') {
        (d, t)
    } else {
        return Err("expected 'YYYY-MM-DD hh:mm:ss'".to_string());
    };

    let date: Vec<&str> = date_part.split('-').collect();
    let time: Vec<&str> = time_part.split(':').collect();

    if date.len() != 3 || time.len() < 2 {
        return Err("invalid date format".to_string());
    }

    let y: i64 = date[0].parse().map_err(|_| "year".to_string())?;
    let mo: i64 = date[1].parse().map_err(|_| "month".to_string())?;
    let d: i64 = date[2].parse().map_err(|_| "day".to_string())?;
    let h: i64 = time[0].parse().map_err(|_| "hours".to_string())?;
    let mi: i64 = time[1].parse().map_err(|_| "minutes".to_string())?;
    let sec: i64 = if time.len() > 2 {
        // Handle fractional seconds: strip after '.' or ','
        let sec_str = time[2].split('.').next().unwrap_or(time[2]);
        let sec_str = sec_str.split(',').next().unwrap_or(sec_str);
        sec_str.parse().unwrap_or(0)
    } else {
        0
    };

    let unix_ts = unix_from_ymdhms(y, mo, d, h, mi, sec)?;
    Ok(FileTime::from_unix_time(unix_ts, 0))
}

/// Parse a `-t` time string: `[[CC]YY]MMDDhhmm[.ss]`.
fn parse_time(s: &str) -> Result<FileTime, String> {
    let (main, secs) = match s.find('.') {
        Some(i) => (&s[..i], s[i + 1..].parse::<i64>().unwrap_or(0)),
        None => (s, 0i64),
    };

    let digits = main.as_bytes();
    let len = digits.len();

    let (y, mo, d, h, mi) = match len {
        8 => (
            1970i64,
            two_digit(&digits[0..2])? as i64,
            two_digit(&digits[2..4])? as i64,
            two_digit(&digits[4..6])? as i64,
            two_digit(&digits[6..8])? as i64,
        ),
        10 => {
            let yy = two_digit(&digits[0..2])? as i64;
            let cc = if yy >= 69 { 1900 } else { 2000 };
            (
                cc + yy,
                two_digit(&digits[2..4])? as i64,
                two_digit(&digits[4..6])? as i64,
                two_digit(&digits[6..8])? as i64,
                two_digit(&digits[8..10])? as i64,
            )
        }
        12 => {
            let y = ((digits[0] - b'0') as i64 * 1000)
                + ((digits[1] - b'0') as i64 * 100)
                + ((digits[2] - b'0') as i64 * 10)
                + (digits[3] - b'0') as i64;
            (
                y,
                two_digit(&digits[4..6])? as i64,
                two_digit(&digits[6..8])? as i64,
                two_digit(&digits[8..10])? as i64,
                two_digit(&digits[10..12])? as i64,
            )
        }
        _ => return Err("expected [[CC]YY]MMDDhhmm[.ss]".to_string()),
    };

    let unix_ts = unix_from_ymdhms(y, mo, d, h, mi, secs)?;
    Ok(FileTime::from_unix_time(unix_ts, 0))
}

/// Parse two ASCII digits into a u32.
fn two_digit(d: &[u8]) -> Result<u32, String> {
    if d.len() != 2 || !d[0].is_ascii_digit() || !d[1].is_ascii_digit() {
        return Err("not a number".to_string());
    }
    Ok(((d[0] - b'0') * 10 + (d[1] - b'0')) as u32)
}

/// Convert year, month, day, hour, minute, second to seconds since the Unix
/// epoch (proleptic Gregorian, 1970 onward).
fn unix_from_ymdhms(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64) -> Result<i64, String> {
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return Err("invalid date".to_string());
    }

    let mut days = 0i64;

    // Count complete years since 1970.
    if y >= 1970 {
        for yy in 1970..y {
            days += if is_leap(yy) { 366 } else { 365 };
        }
    } else {
        for yy in y..1970 {
            days -= if is_leap(yy) { 366 } else { 365 };
        }
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
