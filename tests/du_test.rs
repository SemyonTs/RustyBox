// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use std::os::unix::fs;

use predicates::prelude::*;
use regex::Regex;
use rstest::rstest;

use common::{rb, temp_dir};

// === POSIX du behavior ===
// Specification: https://manpages.ubuntu.com/manpages/questing/man1/du.1posix.html
// - By default: size of each subdirectory
// - Default units: 512-byte blocks
// - -a: Report size of each file
// - -s: Only total sum for each specified file
// - -k: 1024-byte units
// - -H/-L: Follow symlinks

// === Basic du tests ===

#[test]
fn du_default() {
    let dir = temp_dir();
    std::fs::write(dir.path().join("a.txt"), "hello\n").unwrap();

    rb(&["du", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            // Should contain the directory path and a number
            out.contains(dir.path().to_str().unwrap())
        }));
}

#[test]
fn du_current_directory() {
    let dir = temp_dir();
    std::env::set_current_dir(&dir).unwrap();

    rb(&["du"]).assert().success();
}

#[test]
fn du_with_file() {
    let dir = temp_dir();
    let file = dir.path().join("a.txt");
    std::fs::write(&file, "hello\n").unwrap();

    rb(&["du", file.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            out.contains(file.to_str().unwrap())
        }));
}

// === -a (all files) tests ===

#[test]
fn du_all_files() {
    let dir = temp_dir();
    std::fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
    std::fs::write(dir.path().join("b.txt"), "world\n").unwrap();

    rb(&["du", "-a", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            out.contains("a.txt") && out.contains("b.txt")
        }));
}

// === -s (summary) tests ===

#[test]
fn du_summary() {
    let dir = temp_dir();
    std::fs::write(dir.path().join("a.txt"), "hello\n").unwrap();

    // -s should only show the total for the directory
    let output = rb(&["du", "-s", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1);
}

// === -k (1024-byte units) tests ===

#[test]
fn du_k_units() {
    let dir = temp_dir();
    std::fs::write(dir.path().join("a.txt"), "hello\n").unwrap();

    rb(&["du", "-k", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            // Should contain numbers in 1024-byte units
            let re = Regex::new(r"^[0-9]+").unwrap();
            re.is_match(out)
        }));
}

// === Recursive directory tests ===

#[test]
fn du_recursive() {
    let dir = temp_dir();
    let sub = dir.path().join("sub");
    std::fs::create_dir(&sub).unwrap();
    std::fs::write(sub.join("a.txt"), "hello\n").unwrap();

    rb(&["du", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            out.contains(dir.path().to_str().unwrap())
        }));
}

// === Multiple arguments ===

#[test]
fn du_multiple_paths() {
    let dir1 = temp_dir();
    std::fs::write(dir1.path().join("a.txt"), "hello\n").unwrap();

    let dir2 = temp_dir();
    std::fs::write(dir2.path().join("b.txt"), "world\n").unwrap();

    rb(&[
        "du",
        dir1.path().to_str().unwrap(),
        dir2.path().to_str().unwrap(),
    ])
    .assert()
    .success()
    .stdout(predicate::function(|out: &str| {
        out.contains(dir1.path().to_str().unwrap()) && out.contains(dir2.path().to_str().unwrap())
    }));
}

// === Error cases ===

#[test]
fn du_nonexistent_path() {
    rb(&["du", "/nonexistent"]).assert().failure().code(1);
}

#[test]
fn du_follow_symlinks_H() {
    let dir = temp_dir();
    let target = dir.path().join("target");
    std::fs::write(&target, "data\n").unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    rb(&["du", "-H", link.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            out.contains(link.to_str().unwrap())
        }));
}

#[test]
fn du_no_follow_symlinks_default() {
    let dir = temp_dir();
    let target = dir.path().join("target");
    std::fs::write(&target, "data\n").unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    rb(&["du", link.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| out.contains("link")));
}

#[test]
fn du_multiple_paths_with_summary() {
    let dir1 = temp_dir();
    let dir2 = temp_dir();
    rb(&[
        "du",
        "-s",
        dir1.path().to_str().unwrap(),
        dir2.path().to_str().unwrap(),
    ])
    .assert()
    .success()
    .stdout(predicate::function(|out: &str| {
        let lines: Vec<&str> = out.lines().collect();
        lines.len() == 2
    }));
}
