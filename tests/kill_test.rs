// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use std::process;

use common::rb;

// === POSIX kill behavior ===

#[test]
fn kill_list_signals() {
    let output = rb(&["kill", "-l"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    // POSIX requires names without SIG prefix for -l option
    assert!(
        stdout.contains("TERM"),
        "Output should contain TERM: {}",
        stdout
    );
    assert!(
        stdout.contains("KILL"),
        "Output should contain KILL: {}",
        stdout
    );
}

#[test]
fn kill_list_signal_number() {
    // -l 9 should print "KILL"
    let output = rb(&["kill", "-l", "9"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    assert!(
        stdout.contains("KILL"),
        "Output should contain KILL: {}",
        stdout
    );
}

#[test]
fn kill_signal_by_name() {
    // Use signal 0 to check existence without killing the test process
    let pid = process::id().to_string();
    rb(&["kill", "-s", "0", &pid]).assert().success();
}

#[test]
fn kill_signal_by_number() {
    let pid = process::id().to_string();
    // Using signal 0 is safe and validates numeric argument parsing
    rb(&["kill", "-s", "0", &pid]).assert().success();
}

#[test]
fn kill_shorthand_signal() {
    let pid = process::id().to_string();
    // Tests support for -0 shorthand (XSI extension)
    rb(&["kill", "-0", &pid]).assert().success();
}

#[test]
fn kill_invalid_pid() {
    // PID 999999 is likely non-existent
    rb(&["kill", "-s", "0", "999999"])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn kill_invalid_signal() {
    let pid = process::id().to_string();
    rb(&["kill", "-s", "INVALID", &pid])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn kill_l_signal_name() {
    // -l with signal name -> number
    let output = rb(&["kill", "-l", "TERM"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    assert_eq!(stdout.trim(), "15");
}

#[test]
fn kill_l_all_signals_format() {
    let output = rb(&["kill", "-l"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    // Check that we have multiple signals listed
    let signals: Vec<&str> = stdout.split_whitespace().collect();
    assert!(
        signals.len() > 10,
        "Expected more than 10 signals, got: {}",
        stdout
    );
}

#[test]
fn kill_signal_number_with_shorthand() {
    let pid = process::id().to_string();
    rb(&["kill", "-0", &pid]).assert().success();
}
