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
    let date_str = opts.get_str('d').unwrap_or("").to_string();

    // Extract the format string from a leading `+FMT` argument.
    let mut format = String::new();
    for arg in &ctx.optargs {
        if let Some(f) = arg.strip_prefix('+') {
            format = f.to_string();
        }
    }

    // Resolve the base moment in time.
    let base: DateTime<Utc> = if !date_str.is_empty() {
        match parse_date(&date_str) {
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

    // Convert to the requested timezone.
    let dt: DateTime<Local> = if flag_u {
        base.with_timezone(&Utc).into()
    } else {
        base.with_timezone(&Local)
    };

    // Dispatch to the appropriate output format.
    if flag_R {
        println!("{}", dt.to_rfc2822());
    } else if flag_I {
        println!("{}", dt.format("%Y-%m-%dT%H:%M:%S%:z"));
    } else if format.is_empty() {
        // Default format, e.g.: "Mon Jan 01 12:00:00 UTC 2026".
        println!("{}", dt.format("%a %b %e %H:%M:%S %Z %Y"));
    } else {
        println!("{}", apply_format(&dt, &format));
    }

    0
}

/// Render a `DateTime` using a subset of GNU-date `%`-escape sequences.
///
/// Supported specifiers:
///   `%Y`, `%y`, `%m`, `%d`, `%H`, `%M`, `%S`, `%e`, `%b`/`%h`, `%B`,
///   `%a`, `%A`, `%j`, `%U`, `%W`, `%w`, `%Z`, `%z`, `%T`, `%R`, `%D`,
///   `%F`, `%s`, `%n`, `%t`, `%%`.
/// Unknown specifiers are passed through unchanged.
fn apply_format(dt: &DateTime<Local>, fmt: &str) -> String {
    let mut result = String::new();
    let chars: Vec<char> = fmt.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '%' && i + 1 < chars.len() {
            let spec = chars[i + 1];
            let s = match spec {
                'Y' => dt.format("%Y").to_string(),
                'y' => dt.format("%y").to_string(),
                'm' => dt.format("%m").to_string(),
                'd' => dt.format("%d").to_string(),
                'H' => dt.format("%H").to_string(),
                'M' => dt.format("%M").to_string(),
                'S' => dt.format("%S").to_string(),
                'e' => format!("{:2}", dt.day()),
                'b' | 'h' => dt.format("%b").to_string(),
                'B' => dt.format("%B").to_string(),
                'a' => dt.format("%a").to_string(),
                'A' => dt.format("%A").to_string(),
                'j' => dt.format("%j").to_string(),
                'U' => dt.format("%U").to_string(),
                'W' => dt.format("%W").to_string(),
                'w' => dt.format("%w").to_string(),
                'Z' => dt.format("%Z").to_string(),
                'z' => dt.format("%z").to_string(),
                'T' => dt.format("%H:%M:%S").to_string(),
                'R' => dt.format("%H:%M").to_string(),
                'D' => dt.format("%m/%d/%y").to_string(),
                'F' => dt.format("%Y-%m-%d").to_string(),
                's' => dt.timestamp().to_string(),
                'n' => "\n".to_string(),
                't' => "\t".to_string(),
                '%' => "%".to_string(),
                _ => format!("%{}", spec),
            };
            result.push_str(&s);
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
/// Two input forms are accepted:
///   - `@<decimal>` — an explicit Unix timestamp.
///   - `YYYY-MM-DD hh:mm:ss` (or with `T` separator) — a calendar date.
fn parse_date(s: &str) -> Result<i64, String> {
    if let Some(rest) = s.strip_prefix('@') {
        return rest
            .parse::<i64>()
            .map_err(|_| "unix timestamp".to_string());
    }

    // Normalise the `T` separator often used in ISO 8601 datetimes.
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

    let days = days_since_epoch(y, mo, d);
    Ok(days * 86400 + h * 3600 + mi * 60 + sec)
}

/// Return the number of whole days between 1970-01-01 and the given date.
///
/// The calculation is proleptic Gregorian (year 0 and negative years are not
/// handled — the range is 1970 onward).
fn days_since_epoch(y: i64, mo: i64, d: i64) -> i64 {
    let mut days = 0i64;

    // Count complete years.
    for yy in 1970..y {
        days += if (yy % 4 == 0 && yy % 100 != 0) || yy % 400 == 0 {
            366
        } else {
            365
        };
    }

    // Count complete months in the current year.
    let month_days = [0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for mm in 1..mo {
        days += month_days[mm as usize];
        if mm == 2 && (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
            days += 1;
        }
    }

    // Add the remaining days in the current month.
    days += d - 1;

    days
}

register_command!(
    DATE_CMD,
    "date",
    "d:uRI(s)",
    CommandFlags::BIN.bits(),
    date_main
);