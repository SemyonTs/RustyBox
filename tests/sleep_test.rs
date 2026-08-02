// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use rstest::rstest;
use std::time::Instant;

use common::rb;

// === POSIX sleep behavior ===
// Specification: https://manpages.opensuse.org/Leap-16.0/man-pages-posix/sleep.1p.en.html
// - Suspend execution for at least specified number of seconds
// - POSIX only requires integer seconds, but many implementations support fractions

#[test]
fn sleep_one_second() {
    let start = Instant::now();
    rb(&["sleep", "1"]).assert().success();
    let elapsed = start.elapsed();
    assert!(elapsed >= std::time::Duration::from_secs(1));
}

#[test]
fn sleep_fractional_second() {
    let start = Instant::now();
    rb(&["sleep", "0.5"]).assert().success();
    let elapsed = start.elapsed();
    assert!(elapsed >= std::time::Duration::from_millis(500));
}

#[test]
fn sleep_multiple_arguments() {
    // POSIX: multiple arguments are summed
    let start = Instant::now();
    rb(&["sleep", "1", "2"]).assert().success();
    let elapsed = start.elapsed();
    assert!(elapsed >= std::time::Duration::from_secs(3));
}

#[test]
fn sleep_zero() {
    rb(&["sleep", "0"]).assert().success();
}

// === Error cases ===

#[test]
fn sleep_no_args() {
    rb(&["sleep"]).assert().failure().code(1);
}

#[test]
fn sleep_invalid_number() {
    rb(&["sleep", "invalid"]).assert().failure().code(1);
}

#[test]
fn sleep_negative_fails() {
    rb(&["sleep", "-1"]).assert().failure().code(1);
}

#[test]
fn sleep_zero_with_multiple() {
    rb(&["sleep", "0", "0"]).assert().success();
}
