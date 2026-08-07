// =============================================================================
// cal — Print a calendar (POSIX compliant with extensions -j, -y, -m).
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Implementation inspired by Toybox (https://landley.net/toybox/)
// Toybox is copyrighted by Rob Landley, see NOTICE file for license details.
//
// Supported options (extensions):
//   -j   Display Julian days (day of year) instead of day of month.
//   -y   Display the calendar for the current year.
//   -m   Start the week on Monday (instead of Sunday).
//
// POSIX behaviour:
//   - No operands: current month.
//   - One operand: interpreted as year (1..9999), prints all 12 months.
//   - Two operands: month (1..12) and year (1..9999), prints that month.
//
// Calendar transition: Julian until 1752-09-02, Gregorian from 1752-09-14.
// Days 1752-09-03..1752-09-13 are omitted.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use chrono::{Datelike, Local};
use std::fmt::Write;

/// Entry point for the `cal` builtin.
fn cal_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "jym") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("cal: {e}");
            return 1;
        }
    };

    let julian = opts.count('j') > 0;
    let year_only = opts.count('y') > 0;
    let monday_first = opts.count('m') > 0;

    let args: Vec<&str> = ctx.optargs.iter().map(|s| s.as_str()).collect();

    let (month, year) = if year_only {
        let now = Local::now();
        (None, now.year())
    } else {
        match args.len() {
            0 => {
                let now = Local::now();
                (Some(now.month()), now.year())
            }
            1 => {
                let y = args[0].parse::<i32>().unwrap_or_else(|_| {
                    eprintln!("cal: invalid year '{}'", args[0]);
                    std::process::exit(1);
                });
                if !(1..=9999).contains(&y) {
                    eprintln!("cal: year {} out of range (1..9999)", y);
                    return 1;
                }
                (None, y)
            }
            2 => {
                let m = args[0].parse::<u32>().unwrap_or_else(|_| {
                    eprintln!("cal: invalid month '{}'", args[0]);
                    std::process::exit(1);
                });
                let y = args[1].parse::<i32>().unwrap_or_else(|_| {
                    eprintln!("cal: invalid year '{}'", args[1]);
                    std::process::exit(1);
                });
                if !(1..=12).contains(&m) {
                    eprintln!("cal: month {} out of range (1..12)", m);
                    return 1;
                }
                if !(1..=9999).contains(&y) {
                    eprintln!("cal: year {} out of range (1..9999)", y);
                    return 1;
                }
                (Some(m), y)
            }
            _ => {
                eprintln!("cal: too many arguments");
                return 1;
            }
        }
    };

    if let Some(m) = month {
        print_month(m, year, julian, monday_first);
    } else {
        print_year(year, julian, monday_first);
    }

    0
}

// -----------------------------------------------------------------------------
// Calendar core functions (Julian/Gregorian transition).
// -----------------------------------------------------------------------------

/// Determine if a year is a leap year under the given calendar system.
fn is_leap_year(year: i32, julian: bool) -> bool {
    if julian {
        year % 4 == 0
    } else {
        (year % 400 == 0) || (year % 4 == 0 && year % 100 != 0)
    }
}

/// Number of days in a month, taking into account the 1752 transition.
fn days_in_month(year: i32, month: u32) -> u32 {
    if year == 1752 && month == 9 {
        return 19; // 1,2,14-30
    }
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year, year <= 1752) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// Returns a list of valid day numbers for the given month.
/// For September 1752, days 3-13 are omitted.
fn valid_days(year: i32, month: u32) -> Vec<u32> {
    if year == 1752 && month == 9 {
        let mut days = Vec::with_capacity(19);
        days.extend(1..=2);
        days.extend(14..=30);
        days
    } else {
        (1..=days_in_month(year, month)).collect()
    }
}

/// Returns the day of the week (0 = Sunday, 1 = Monday, ...) for a given date.
/// Uses Julian calendar for dates up to 1752-09-02, Gregorian from 1752-09-14.
fn day_of_week(year: i32, month: u32, day: u32) -> u32 {
    let use_julian = if year < 1752 || (year == 1752 && (month < 9 || (month == 9 && day <= 2))) {
        true
    } else {
        false
    };

    if use_julian {
        // Julian calendar: count days from 1 Jan 1.
        let mut days: i64 = 0;
        for y in 1..year {
            days += if is_leap_year(y, true) { 366 } else { 365 };
        }
        for m in 1..month {
            days += days_in_month(year, m) as i64;
        }
        days += day as i64 - 1;
        // 1 Jan 1 was Monday. days=0 -> Monday.
        // Map to Sunday=0: (days + 1) % 7
        ((days + 1) % 7) as u32
    } else {
        // Tomohiko Sakamoto's algorithm for Gregorian.
        // Returns 0=Sunday, 1=Monday, ..., 6=Saturday.
        let m = month as i32;
        let d = day as i32;

        let (y, m_adj) = if m < 3 { (year - 1, m + 12) } else { (year, m) };

        let t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
        let idx = ((m_adj - 1) % 12) as usize;
        let dow = (y + y / 4 - y / 100 + y / 400 + t[idx] + d) % 7;
        dow as u32
    }
}

/// Return the day of year (1..366) for a given date.
fn day_of_year(year: i32, month: u32, day: u32) -> u32 {
    let mut doy = 0;
    for m in 1..month {
        doy += days_in_month(year, m);
    }
    doy + day
}

// -----------------------------------------------------------------------------
// Printing functions.
// -----------------------------------------------------------------------------

/// Print a single month calendar.
fn print_month(month: u32, year: i32, julian: bool, monday_first: bool) {
    let days = valid_days(year, month);
    if days.is_empty() {
        return;
    }
    let first_day = day_of_week(year, month, days[0]);

    let offset = if monday_first {
        (first_day + 6) % 7 // convert Sunday=0 to Monday=0
    } else {
        first_day
    };

    // Header.
    let month_name = match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => unreachable!(),
    };
    let header = format!("{} {}", month_name, year);
    let width = 20;
    let padding = (width - header.len()) / 2;
    println!("{:>padding$}{}", "", header);

    // Weekday names.
    let weekdays = if monday_first {
        ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"]
    } else {
        ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"]
    };
    for wd in &weekdays {
        print!(" {:2}", wd);
    }
    println!();

    // Print days.
    let mut printed = 0;
    for _ in 0..offset {
        print!("   ");
        printed += 1;
    }
    for &d in &days {
        if printed % 7 == 0 && printed > 0 {
            println!();
        }
        let num = if julian {
            day_of_year(year, month, d)
        } else {
            d
        };
        print!("{:>3}", num);
        printed += 1;
    }
    println!();
}

/// Print a whole year (12 months) in four rows of three months.
fn print_year(year: i32, julian: bool, monday_first: bool) {
    let mut month_lines: Vec<Vec<String>> = Vec::with_capacity(12);
    for m in 1..=12 {
        let lines = build_month_lines(m, year, julian, monday_first);
        month_lines.push(lines);
    }

    let mut row = 0;
    while row < 12 {
        let end = std::cmp::min(row + 3, 12);
        let max_lines = month_lines[row..end]
            .iter()
            .map(|v| v.len())
            .max()
            .unwrap_or(0);
        for line_idx in 0..max_lines {
            let mut row_str = String::new();
            for col in 0..3 {
                let m_idx = row + col;
                if m_idx >= 12 {
                    break;
                }
                let lines = &month_lines[m_idx];
                let line = if line_idx < lines.len() {
                    &lines[line_idx]
                } else {
                    ""
                };
                write!(row_str, "{:<20}", line).unwrap();
                if col < 2 && m_idx + 1 < 12 {
                    row_str.push(' ');
                }
            }
            println!("{}", row_str);
        }
        row += 3;
        if row < 12 {
            println!();
        }
    }
}

/// Build the lines for a single month (header + day grid), each line 20 chars.
fn build_month_lines(month: u32, year: i32, julian: bool, monday_first: bool) -> Vec<String> {
    let days = valid_days(year, month);
    if days.is_empty() {
        return Vec::new();
    }
    let first_day = day_of_week(year, month, days[0]);
    let offset = if monday_first {
        (first_day + 6) % 7
    } else {
        first_day
    };

    let mut lines = Vec::new();

    // Header.
    let month_name = match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => unreachable!(),
    };
    let header = format!("{} {}", month_name, year);
    let width = 20;
    let padding = (width - header.len()) / 2;
    let mut header_line = format!("{:>padding$}{}", "", header);
    if header_line.len() < width {
        header_line.push_str(&" ".repeat(width - header_line.len()));
    }
    lines.push(header_line);

    // Weekday names.
    let weekdays = if monday_first {
        ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"]
    } else {
        ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"]
    };
    let mut wd_line = String::with_capacity(20);
    for (i, wd) in weekdays.iter().enumerate() {
        if i == 0 {
            write!(wd_line, "{:2}", wd).unwrap();
        } else {
            write!(wd_line, " {:2}", wd).unwrap();
        }
    }
    if wd_line.len() < width {
        wd_line.push_str(&" ".repeat(width - wd_line.len()));
    }
    lines.push(wd_line);

    // Day grid.
    let mut day_line = String::with_capacity(20);
    let mut printed = 0;
    for _ in 0..offset {
        day_line.push_str("   ");
        printed += 1;
    }
    for &d in &days {
        if printed % 7 == 0 && printed > 0 {
            if day_line.len() < width {
                day_line.push_str(&" ".repeat(width - day_line.len()));
            }
            lines.push(day_line);
            day_line = String::with_capacity(20);
        }
        let num = if julian {
            day_of_year(year, month, d)
        } else {
            d
        };
        write!(day_line, "{:>3}", num).unwrap();
        printed += 1;
    }
    if !day_line.is_empty() {
        if day_line.len() < width {
            day_line.push_str(&" ".repeat(width - day_line.len()));
        }
        lines.push(day_line);
    }

    lines
}

// -----------------------------------------------------------------------------
// Command registration.
// -----------------------------------------------------------------------------
register_command!(
    CAL_CMD,
    "cal",
    "jym",
    CommandFlags::BIN.bits(),
    cal_main,
    description = "Print a calendar",
    help = "\
OPTIONS:
-j   Display Julian days (day of year) instead of day of month.
-y   Display the calendar for the current year.
-m   Start the week on Monday (instead of Sunday).

USAGE:
    cal [month] [year]
    cal -y
    cal -j month year
"
);
