// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use regex::Regex;
use rstest::rstest;

use common::rb;

// === POSIX uname behavior ===
// Specification: https://manpages.ubuntu.com/manpages/xenial/en/man1/uname.1posix.html
// - Print system information
// - -s: Kernel name (default)
// - -n: Node name
// - -r: Kernel release
// - -v: Kernel version
// - -m: Machine
// - -a: All

#[test]
fn uname_default() {
    let output = rb(&["uname"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout_str = String::from_utf8(output).unwrap();
    let stdout = stdout_str.trim();
    // Should be a non-empty string (typically "Linux")
    assert!(!stdout.is_empty());
}

#[test]
fn uname_s_kernel_name() {
    let output = rb(&["uname", "-s"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout_str = String::from_utf8(output).unwrap();
    let stdout = stdout_str.trim();
    assert!(!stdout.is_empty());
}

#[test]
fn uname_n_node_name() {
    let output = rb(&["uname", "-n"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout_str = String::from_utf8(output).unwrap();
    let stdout = stdout_str.trim();
    assert!(!stdout.is_empty());
}

#[test]
fn uname_r_kernel_release() {
    let output = rb(&["uname", "-r"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout_str = String::from_utf8(output).unwrap();
    let stdout = stdout_str.trim();

    assert!(!stdout.is_empty(), "uname -r output is empty");
    assert!(
        stdout.chars().any(|c| c.is_ascii_digit()),
        "uname -r output contains no digits: {}",
        stdout
    );
}

#[test]
fn uname_v_kernel_version() {
    let output = rb(&["uname", "-v"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout_str = String::from_utf8(output).unwrap();
    let stdout = stdout_str.trim();
    assert!(!stdout.is_empty());
}

#[test]
fn uname_m_machine() {
    let output = rb(&["uname", "-m"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout_str = String::from_utf8(output).unwrap();
    let stdout = stdout_str.trim();
    // Should be something like "x86_64" or "aarch64"
    assert!(!stdout.is_empty());
}

#[test]
fn uname_a_all() {
    let output = rb(&["uname", "-a"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout_str = String::from_utf8(output).unwrap();
    let stdout = stdout_str.trim();
    // Should contain multiple fields
    let fields: Vec<&str> = stdout.split_whitespace().collect();
    assert!(fields.len() >= 4);
}

#[test]
fn uname_combined_options() {
    let output = rb(&["uname", "-sr"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout_str = String::from_utf8(output).unwrap();
    let stdout = stdout_str.trim();
    // Should contain both kernel name and release
    let fields: Vec<&str> = stdout.split_whitespace().collect();
    assert!(fields.len() >= 2);
}

// === Error cases ===

#[test]
fn uname_invalid_option() {
    rb(&["uname", "-x"]).assert().failure().code(1);
}

#[test]
fn uname_a_all_fields() {
    let output = rb(&["uname", "-a"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout_str = String::from_utf8(output).unwrap();
    let stdout = stdout_str.trim();
    let parts: Vec<&str> = stdout.split_whitespace().collect();
    assert!(parts.len() >= 5);
}

#[test]
fn uname_sr_combined() {
    rb(&["uname", "-sr"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            let parts: Vec<&str> = out.split_whitespace().collect();
            parts.len() == 2
        }));
}
