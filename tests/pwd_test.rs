// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use assert_cmd::Command;
use predicates::prelude::*;
use rstest::rstest;
use std::fs;
use std::os::unix::fs::symlink;

use common::{rb, temp_dir};

// === POSIX pwd behavior ===
// Specification: https://manpages.ubuntu.com/manpages/xenial/en/man1/pwd.1posix.html

#[test]
fn pwd_default() {
    let current = std::env::current_dir().unwrap();
    let output = rb(&["pwd"]).assert().success().get_output().stdout.clone();

    let stdout_str = String::from_utf8(output).unwrap();
    let stdout = stdout_str.trim();
    assert_eq!(stdout, current.to_str().unwrap());
}

#[test]
fn pwd_absolute_path() {
    let output = rb(&["pwd"]).assert().success().get_output().stdout.clone();

    let stdout_str = String::from_utf8(output).unwrap();
    let stdout = stdout_str.trim();
    assert!(stdout.starts_with('/'));
}

#[test]
fn pwd_no_dot_or_dotdot() {
    let output = rb(&["pwd"]).assert().success().get_output().stdout.clone();

    let stdout_str = String::from_utf8(output).unwrap();
    let stdout = stdout_str.trim();
    assert!(!stdout.contains("/./"));
    assert!(!stdout.contains("/../"));
}

// === -L logical path tests ===

#[test]
fn pwd_logical_via_symlink() {
    let dir = temp_dir();
    let real = dir.path().join("real");
    fs::create_dir(&real).unwrap();

    let link = dir.path().join("link");
    symlink(&real, &link).unwrap();

    // Emulate shell behavior:
    // 1. Set CWD for the child process only (thread-safe).
    // 2. Set PWD environment variable to the logical path.
    Command::cargo_bin("rustybox")
        .unwrap()
        .args(&["pwd", "-L"])
        .current_dir(&link)
        .env("PWD", link.to_str().unwrap())
        .assert()
        .success()
        .stdout(predicate::str::contains("link"));
}

// === -P physical path tests ===

#[test]
fn pwd_physical_via_symlink() {
    let dir = temp_dir();
    let real = dir.path().join("real");
    fs::create_dir(&real).unwrap();

    let link = dir.path().join("link");
    symlink(&real, &link).unwrap();

    Command::cargo_bin("rustybox")
        .unwrap()
        .args(&["pwd", "-P"])
        .current_dir(&link)
        .env("PWD", link.to_str().unwrap()) // Even with PWD set, -P must ignore it
        .assert()
        .success()
        .stdout(predicate::str::contains("real"))
        .stdout(predicate::str::contains("link").not());
}

// === Error cases ===

#[test]
fn pwd_too_many_args() {
    rb(&["pwd", "extra"]).assert().failure().code(1);
}

#[test]
fn pwd_default_logical() {
    let dir = temp_dir();
    let real = dir.path().join("real");
    fs::create_dir(&real).unwrap();

    let link = dir.path().join("link");
    symlink(&real, &link).unwrap();

    // Default behavior without flags should be -L (logical)
    Command::cargo_bin("rustybox")
        .unwrap()
        .args(&["pwd"])
        .current_dir(&link)
        .env("PWD", link.to_str().unwrap())
        .assert()
        .success()
        .stdout(predicate::str::contains("link"));
}
