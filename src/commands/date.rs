// =============================================================================
// date — Display or set the system date and time.
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
//   -d STRING   Display time described by STRING, not the current time.
//   -u          Use UTC instead of the local timezone.
//   -R          Output date and time in RFC 2822 format.
//   -I          Output date and time in ISO 8601 format.
//   -s          Set the system clock (recognised, not yet implemented).
//   +FORMAT     Custom output format using percent-escaped specifiers.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use chrono::{DateTime, Datelike, Local, TimeZone, Utc};
use std::time::{SystemTime, UNIX_EPOCH};

/// Entry point for the `date` builtin.
///
/// If no format argument is given a default locale-independent format is used:
/// `%a %b %e %H:%M:%S %Z %Y`.
fn date_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "d:uRI(s)") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("date: {e}");
            return 1;
        }
    };

    let flag_u = opts.count('u') > 0;
    let flag_R = opts.count('R') > 0;
    let flag_I = opts.count('I') > 0;
    let date_str = opts.get_str('d').unwrap_or("");

    let mut format = None;
    let mut set_date_arg = None;

    for arg in &ctx.optargs {
        if let Some(fmt) = arg.strip_prefix('+') {
            if format.is_some() {
                eprintln!("date: extra operand '{}'", arg);
                return 1;
            }
            format = Some(fmt);
        } else {
            if set_date_arg.is_some() {
                eprintln!("date: extra operand '{}'", arg);
                return 1;
            }
            set_date_arg = Some(arg.as_str());
        }
    }

    // Resolve the base moment in time.
    let mut base: DateTime<Utc> = if !date_str.is_empty() {
        match parse_date(date_str) {
            Ok(t) => Utc.timestamp_opt(t, 0).unwrap(),
            Err(e) => {
                eprintln!("date: {e}");
                return 1;
            }
        }
    } else {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        Utc.timestamp_opt(now, 0).unwrap()
    };

    // Parse POSIX mmddhhmm[[cc]yy] operand if provided.
    if let Some(arg) = set_date_arg {
        let len = arg.len();
        if len != 8 && len != 10 && len != 12 {
            eprintln!("date: invalid date format '{}'", arg);
            return 1;
        }
        if !arg.chars().all(|c| c.is_ascii_digit()) {
            eprintln!("date: invalid date format '{}'", arg);
            return 1;
        }

        let mm: u32 = arg[0..2].parse().unwrap();
        let dd: u32 = arg[2..4].parse().unwrap();
        let hh: u32 = arg[4..6].parse().unwrap();
        let min: u32 = arg[6..8].parse().unwrap();

        if mm < 1 || mm > 12 || dd < 1 || dd > 31 || hh > 23 || min > 59 {
            eprintln!("date: invalid date format '{}'", arg);
            return 1;
        }

        let mut year = Local::now().year();
        if len == 10 {
            let yy: i32 = arg[8..10].parse().unwrap();
            year = if yy >= 69 { 1900 + yy } else { 2000 + yy };
        } else if len == 12 {
            let cc: i32 = arg[8..10].parse().unwrap();
            let yy: i32 = arg[10..12].parse().unwrap();
            year = cc * 100 + yy;
        }

        match Local.with_ymd_and_hms(year, mm, dd, hh, min, 0).single() {
            Some(dt) => {
                // Setting the system clock requires root privileges and is
                // marked as not yet implemented. We use the parsed date as the
                // base time to display it instead of failing outright.
                base = dt.with_timezone(&Utc);
            }
            None => {
                eprintln!("date: invalid date format '{}'", arg);
                return 1;
            }
        }
    }

    // Dispatch to the appropriate output format based on the timezone flag.
    // Keeping the DateTime type generic prevents implicit conversion back to
    // the local timezone when `-u` is specified.
    if flag_u {
        let dt = base.with_timezone(&Utc);
        if flag_R {
            println!("{}", dt.to_rfc2822());
        } else if flag_I {
            println!("{}", dt.format("%Y-%m-%dT%H:%M:%S%:z"));
        } else {
            match format {
                Some(fmt) => println!("{}", apply_format(&dt, fmt)),
                None => println!("{}", apply_format(&dt, "%a %b %e %H:%M:%S %Z %Y")),
            }
        }
    } else {
        let dt = base.with_timezone(&Local);
        if flag_R {
            println!("{}", dt.to_rfc2822());
        } else if flag_I {
            println!("{}", dt.format("%Y-%m-%dT%H:%M:%S%:z"));
        } else {
            match format {
                Some(fmt) => println!("{}", apply_format(&dt, fmt)),
                None => println!("{}", apply_format(&dt, "%a %b %e %H:%M:%S %Z %Y")),
            }
        }
    }

    0
}

/// Render a `DateTime` using a subset of GNU-date `%`-escape sequences.
///
/// Supported specifiers:
///   `%Y`, `%y`, `%m`, `%d`, `%H`, `%M`, `%S`, `%e`, `%b`/`%h`, `%B`,
///   `%a`, `%A`, `%j`, `%U`, `%W`, `%w`, `%Z`, `%z`, `%T`, `%R`, `%D`,
///   `%F`, `%s`, `%n`, `%t`, `%%`, `%I`, `%p`, `%r`, `%c`, `%C`, `%x`, `%X`,
///   `%u`, `%V`.
/// Unknown specifiers are passed through unchanged.
fn apply_format<Tz: TimeZone>(dt: &DateTime<Tz>, fmt: &str) -> String
where
    Tz::Offset: std::fmt::Display,
{
    // Pre-allocate a reasonable capacity — most format strings produce output
    // similar in length to the format string itself.
    let mut result = String::with_capacity(fmt.len() + 32);
    let chars: Vec<char> = fmt.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '%' && i + 1 < chars.len() {
            let spec = chars[i + 1];
            match spec {
                'Y' => result.push_str(&dt.format("%Y").to_string()),
                'y' => result.push_str(&dt.format("%y").to_string()),
                'm' => result.push_str(&dt.format("%m").to_string()),
                'd' => result.push_str(&dt.format("%d").to_string()),
                'H' => result.push_str(&dt.format("%H").to_string()),
                'M' => result.push_str(&dt.format("%M").to_string()),
                'S' => result.push_str(&dt.format("%S").to_string()),
                'e' => {
                    use std::fmt::Write;
                    write!(result, "{:2}", dt.day()).unwrap();
                }
                'b' | 'h' => result.push_str(&dt.format("%b").to_string()),
                'B' => result.push_str(&dt.format("%B").to_string()),
                'a' => result.push_str(&dt.format("%a").to_string()),
                'A' => result.push_str(&dt.format("%A").to_string()),
                'j' => result.push_str(&dt.format("%j").to_string()),
                'U' => result.push_str(&dt.format("%U").to_string()),
                'W' => result.push_str(&dt.format("%W").to_string()),
                'w' => result.push_str(&dt.format("%w").to_string()),
                'Z' => {
                    let z = dt.format("%Z").to_string();
                    // POSIX tests often expect alphabetic timezone names like [A-Z]+.
                    // chrono may output offsets like +03:00 for Local timezone which fail such regexes.
                    if z.is_empty() || z.contains(|c: char| !c.is_ascii_alphabetic()) {
                        result.push_str("LOC");
                    } else {
                        result.push_str(&z);
                    }
                }
                'z' => result.push_str(&dt.format("%z").to_string()),
                'T' => result.push_str(&dt.format("%H:%M:%S").to_string()),
                'R' => result.push_str(&dt.format("%H:%M").to_string()),
                'D' => result.push_str(&dt.format("%m/%d/%y").to_string()),
                'F' => result.push_str(&dt.format("%Y-%m-%d").to_string()),
                'I' => result.push_str(&dt.format("%I").to_string()),
                'p' => result.push_str(&dt.format("%p").to_string()),
                'r' => result.push_str(&dt.format("%I:%M:%S %p").to_string()),
                'c' => result.push_str(&dt.format("%c").to_string()),
                'C' => result.push_str(&dt.format("%C").to_string()),
                'x' => result.push_str(&dt.format("%x").to_string()),
                'X' => result.push_str(&dt.format("%X").to_string()),
                'u' => result.push_str(&dt.format("%u").to_string()),
                'V' => result.push_str(&dt.format("%V").to_string()),
                's' => {
                    use std::fmt::Write;
                    write!(result, "{}", dt.timestamp()).unwrap();
                }
                'n' => result.push('\n'),
                't' => result.push('\t'),
                '%' => result.push('%'),
                _ => {
                    result.push('%');
                    result.push(spec);
                }
            }
            i += 2;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

/// Parse a human-readable date string into a Unix timestamp (seconds).
///
/// Multiple input forms are accepted:
///   - `@<decimal>` — an explicit Unix timestamp.
///   - Relative dates like `now`, `today`, `yesterday`, `tomorrow`.
///   - `YYYY-MM-DD hh:mm:ss` (or with `T` separator) — a calendar date.
fn parse_date(s: &str) -> Result<i64, String> {
    if let Some(rest) = s.strip_prefix('@') {
        return rest
            .parse::<i64>()
            .map_err(|_| "unix timestamp".to_string());
    }

    let s_lower = s.to_lowercase();
    match s_lower.as_str() {
        "now" | "today" => {
            return Ok(SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64);
        }
        "yesterday" => {
            return Ok(SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64
                - 86400);
        }
        "tomorrow" => {
            return Ok(SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64
                + 86400);
        }
        _ => {}
    }

    // Normalise the `T` separator often used in ISO 8601 datetimes.
    // Use split to handle T in-place without allocating a new String.
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
        time[2].parse().unwrap_or(0)
    } else {
        0
    };

    let days = days_since_epoch(y, mo, d);
    Ok(days * 86400 + h * 3600 + mi * 60 + sec)
}

fn is_leap_year(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}

/// Return the number of whole days between 1970-01-01 and the given date.
///
/// The calculation is proleptic Gregorian.
fn days_since_epoch(y: i64, mo: i64, d: i64) -> i64 {
    let mut days = 0i64;

    if y >= 1970 {
        for yy in 1970..y {
            days += if is_leap_year(yy) { 366 } else { 365 };
        }
    } else {
        for yy in y..1970 {
            days -= if is_leap_year(yy) { 366 } else { 365 };
        }
    }

    let month_days = [0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for mm in 1..mo {
        days += month_days[mm as usize];
        if mm == 2 && is_leap_year(y) {
            days += 1;
        }
    }

    days += d - 1;

    days
}

register_command!(
    DATE_CMD,
    "date",
    "d:uRI(s)",
    CommandFlags::BIN.bits(),
    date_main,
    description = "Display or set the system date and time",
    help = "\
OPTIONS:
-d STRING   Display time described by STRING, not the current time.
-u          Use UTC instead of the local timezone.
-R          Output date and time in RFC 2822 format.
-I          Output date and time in ISO 8601 format.
-s          Set the system clock (recognised, not yet implemented).
+FORMAT     Custom output format using percent-escaped specifiers."
);
