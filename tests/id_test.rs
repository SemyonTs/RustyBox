// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use regex::Regex;
use rstest::rstest;

use common::rb;

// === POSIX id behavior ===
// Specification: https://manpages.opensuse.org/Leap-16.0/man-pages-posix/id.1p.en.html
// - Write user and group IDs and names
// - -u: Effective user ID only
// - -g: Effective group ID only
// - -G: All group IDs
// - -n: Print names instead of numbers
// - -r: Real IDs instead of effective

#[test]
fn id_default() {
    let output = rb(&["id"]).assert().success().get_output().stdout.clone();

    let stdout = String::from_utf8(output).unwrap();
    // Trim whitespace/newlines to ensure the regex matches the content correctly
    let stdout_trimmed = stdout.trim();

    // Should match pattern: uid=N(name) gid=N(name) groups=N(name),...
    let re = Regex::new(r"^uid=\d+\([^)]+\) gid=\d+\([^)]+\) groups=.*$").unwrap();
    assert!(
        re.is_match(stdout_trimmed),
        "Failed to match id output: '{}'",
        stdout_trimmed
    );
}

#[test]
fn id_u_effective_user() {
    let output = rb(&["id", "-u"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    let uid = stdout.trim().parse::<u32>().unwrap();
    assert!(uid > 0); // Should be a valid UID
}

#[test]
fn id_g_effective_group() {
    let output = rb(&["id", "-g"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    let gid = stdout.trim().parse::<u32>().unwrap();
    assert!(gid > 0);
}

#[test]
fn id_G_all_groups() {
    let output = rb(&["id", "-G"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    // Should contain at least one number
    let numbers: Vec<&str> = stdout.split_whitespace().collect();
    assert!(!numbers.is_empty());
    for n in numbers {
        n.parse::<u32>().unwrap();
    }
}

#[test]
fn id_u_n_user_name() {
    let output = rb(&["id", "-u", "-n"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    let name = stdout.trim();
    assert!(!name.is_empty());
    // Should be a valid username (not a number)
    assert!(name.parse::<u32>().is_err());
}

#[test]
fn id_g_n_group_name() {
    let output = rb(&["id", "-g", "-n"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    let name = stdout.trim();
    assert!(!name.is_empty());
    assert!(name.parse::<u32>().is_err());
}

#[test]
fn id_G_n_group_names() {
    let output = rb(&["id", "-G", "-n"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    let names: Vec<&str> = stdout.split_whitespace().collect();
    assert!(!names.is_empty());
    for name in names {
        assert!(name.parse::<u32>().is_err());
    }
}

#[test]
fn id_r_real_user() {
    let output = rb(&["id", "-u", "-r"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    stdout.trim().parse::<u32>().unwrap();
}

// === Error cases ===

#[test]
fn id_invalid_option() {
    rb(&["id", "-x"]).assert().failure().code(1);
}

#[test]
fn id_user_name() {
    let current_user = std::env::var("USER").unwrap_or("root".to_string());
    rb(&["id", &current_user])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| out.contains("uid=")));
}

#[test]
fn id_real_user_with_name() {
    rb(&["id", "-u", "-r", "-n"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            let name = out.trim();
            !name.is_empty() && name.parse::<u32>().is_err()
        }));
}

#[test]
fn id_all_groups_with_names() {
    rb(&["id", "-G", "-n"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            out.split_whitespace().all(|g| g.parse::<u32>().is_err())
        }));
}
