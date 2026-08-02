// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use regex::Regex;
use rstest::rstest;

use common::rb;

// === POSIX df behavior ===
// Specification: https://manpages.ubuntu.com/manpages/resolute/en/man1/df.1posix.html
// - Write amount of available space for file systems
// - Default: 512-byte units
// - -k: 1024-byte units
// - -P: POSIX output format
// - -t: Include total allocated-space figures

// === Basic df tests ===

#[test]
fn df_default() {
    let output = rb(&["df"]).assert().success().get_output().stdout.clone();

    let stdout = String::from_utf8(output).unwrap();
    // Should have at least a header and some data
    assert!(stdout.lines().count() >= 2);
}

#[test]
fn df_with_path() {
    let output = rb(&["df", "/"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    assert!(stdout.contains("Filesystem"));
}

// === -k 1024-byte units tests ===

#[test]
fn df_k_option() {
    rb(&["df", "-k", "/"]).assert().success();
    // Just verify it doesn't fail
}

// === -P POSIX format tests ===

#[test]
fn df_p_posix_format() {
    let output = rb(&["df", "-P", "/"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    // POSIX format header: "Filesystem 512-blocks Used Available Capacity Mounted on"
    // Or with -k: "Filesystem 1024-blocks Used Available Capacity Mounted on"
    assert!(
        stdout.contains("Filesystem") && stdout.contains("512-blocks")
            || stdout.contains("1024-blocks")
    );
}

#[test]
fn df_p_with_k() {
    let output = rb(&["df", "-P", "-k", "/"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    assert!(stdout.contains("1024-blocks"));
}

// === -t total space tests ===

#[test]
fn df_t_option() {
    rb(&["df", "-t", "/"]).assert().success();
    // Just verify it doesn't fail
}

// === Multiple file systems ===

#[test]
fn df_multiple_paths() {
    rb(&["df", "/", "/usr"]).assert().success();
    // Should handle multiple arguments
}

// === Error cases ===

#[test]
fn df_nonexistent_path() {
    rb(&["df", "/nonexistent"]).assert().failure().code(1);
}

#[test]
fn df_inode_usage() {
    rb(&["df", "-i", "/"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| out.contains("IUsed")));
}

#[test]
fn df_with_path_and_human_readable() {
    rb(&["df", "-k", "-P", "/"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| out.contains("1024-blocks")));
}

#[test]
fn df_multiple_paths_with_total() {
    rb(&["df", "-t", "/", "/usr"]).assert().success();
}
