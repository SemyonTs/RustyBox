// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use regex::Regex;
use rstest::rstest;

use common::rb;

// === POSIX date behavior ===
// Specification: https://manpages.opensuse.org/Leap-16.0/man-pages-posix/date.1p.en.html
// - Write current date and time by default
// - -u: Use UTC
// - +format: Format output with conversion specifiers
// - mmddhhmm[[cc]yy]: Set date (not tested here)

// === Default output tests ===

#[test]
fn date_default_format() {
    // Default format: %a %b %e %H:%M:%S %Z %Y
    let output = rb(&["date"]).assert().success().get_output().stdout.clone();

    let stdout = String::from_utf8(output).unwrap().trim().to_string();

    // Should match pattern like "Mon Jan  1 12:34:56 UTC 2024"
    let re = Regex::new(
        r"^[A-Za-z]{3} [A-Za-z]{3} [ 0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2} [A-Za-z0-9]+ [0-9]{4}$",
    )
    .unwrap();
    assert!(re.is_match(&stdout));
}

// === -u UTC tests ===

#[test]
fn date_utc() {
    let output = rb(&["date", "-u"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    // Should contain "UTC" or "GMT"
    assert!(stdout.contains("UTC") || stdout.contains("GMT"));
}

// === Format specifier tests ===

#[test]
fn date_format_year() {
    rb(&["date", "+%Y"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            out.trim().parse::<u32>().is_ok() && out.trim().len() == 4
        }));
}

#[test]
fn date_format_month() {
    rb(&["date", "+%m"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            let val = out.trim().parse::<u32>().unwrap_or(0);
            (1..=12).contains(&val)
        }));
}

#[test]
fn date_format_day() {
    rb(&["date", "+%d"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            let val = out.trim().parse::<u32>().unwrap_or(0);
            (1..=31).contains(&val)
        }));
}

#[test]
fn date_format_hour() {
    rb(&["date", "+%H"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            let val = out.trim().parse::<u32>().unwrap_or(0);
            (0..=23).contains(&val)
        }));
}

#[test]
fn date_format_minute() {
    rb(&["date", "+%M"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            let val = out.trim().parse::<u32>().unwrap_or(0);
            (0..=59).contains(&val)
        }));
}

#[test]
fn date_format_second() {
    rb(&["date", "+%S"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            let val = out.trim().parse::<u32>().unwrap_or(0);
            (0..=59).contains(&val)
        }));
}

#[test]
fn date_format_weekday_abbrev() {
    rb(&["date", "+%a"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            matches!(
                out.trim(),
                "Mon" | "Tue" | "Wed" | "Thu" | "Fri" | "Sat" | "Sun"
            )
        }));
}

#[test]
fn date_format_weekday_full() {
    rb(&["date", "+%A"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            matches!(
                out.trim(),
                "Monday" | "Tuesday" | "Wednesday" | "Thursday" | "Friday" | "Saturday" | "Sunday"
            )
        }));
}

#[test]
fn date_format_month_abbrev() {
    rb(&["date", "+%b"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            matches!(
                out.trim(),
                "Jan"
                    | "Feb"
                    | "Mar"
                    | "Apr"
                    | "May"
                    | "Jun"
                    | "Jul"
                    | "Aug"
                    | "Sep"
                    | "Oct"
                    | "Nov"
                    | "Dec"
            )
        }));
}

#[test]
fn date_format_month_full() {
    rb(&["date", "+%B"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            matches!(
                out.trim(),
                "January"
                    | "February"
                    | "March"
                    | "April"
                    | "May"
                    | "June"
                    | "July"
                    | "August"
                    | "September"
                    | "October"
                    | "November"
                    | "December"
            )
        }));
}

#[test]
fn date_format_iso_week() {
    rb(&["date", "+%W"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            out.trim().parse::<u32>().is_ok()
        }));
}

#[test]
fn date_format_percent() {
    rb(&["date", "+%%"])
        .assert()
        .success()
        .stdout(predicate::eq("%\n"));
}

#[test]
fn date_format_time_12hour() {
    rb(&["date", "+%I"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            let val = out.trim().parse::<u32>().unwrap_or(0);
            (1..=12).contains(&val)
        }));
}

#[test]
fn date_format_am_pm() {
    rb(&["date", "+%p"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            matches!(out.trim(), "AM" | "PM")
        }));
}

// === Combined format tests ===

#[test]
fn date_format_iso_8601() {
    rb(&["date", "+%Y-%m-%d"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            let re = Regex::new(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}\n$").unwrap();
            re.is_match(out)
        }));
}

#[test]
fn date_format_time() {
    rb(&["date", "+%T"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            let re = Regex::new(r"^[0-9]{2}:[0-9]{2}:[0-9]{2}\n$").unwrap();
            re.is_match(out)
        }));
}

// === -d date string tests ===

#[test]
fn date_d_timestamp() {
    // @0 = 1970-01-01 00:00:00 UTC
    rb(&["date", "-d", "@0", "+%Y-%m-%d %H:%M:%S"])
        .assert()
        .success()
        .stdout(predicate::eq("1970-01-01 00:00:00\n"));
}

#[test]
fn date_d_iso_format() {
    rb(&["date", "-d", "2024-01-15T12:30:45", "+%Y-%m-%d %H:%M:%S"])
        .assert()
        .success()
        .stdout(predicate::eq("2024-01-15 12:30:45\n"));
}

// === Error cases ===

#[test]
fn date_invalid_format() {
    rb(&["date", "+%z"]).assert().success(); // Invalid specifiers are passed through
}

#[test]
fn date_invalid_date_string() {
    rb(&["date", "-d", "invalid", "+%Y"])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn date_format_seconds_since_epoch() {
    rb(&["date", "+%s"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            out.trim().parse::<u64>().is_ok()
        }));
}

#[test]
fn date_format_timezone() {
    rb(&["date", "+%Z"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| !out.trim().is_empty()));
}

#[test]
fn date_d_relative_date() {
    rb(&["date", "-d", "now", "+%Y-%m-%d"]).assert().success();
}

#[test]
fn date_d_epoch_with_timezone() {
    rb(&["date", "-d", "@0", "-u", "+%Y-%m-%d %H:%M:%S"])
        .assert()
        .success()
        .stdout(predicate::eq("1970-01-01 00:00:00\n"));
}
