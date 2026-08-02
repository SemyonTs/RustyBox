// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use rstest::rstest;
use std::fs;

use common::{rb, temp_dir, temp_file_with};

// === POSIX rm behavior ===
// Specification: https://manpages.ubuntu.com/manpages/xenial/en/man1/rm.1posix.html
// - Remove directory entries
// - -f: Force (ignore nonexistent, no prompts)
// - -i: Interactive prompt
// - -r, -R: Recursive remove directories
// - -v: Verbose

#[test]
fn rm_remove_file() {
    let (_dir, path) = temp_file_with("content\n");
    rb(&["rm", path.to_str().unwrap()]).assert().success();

    assert!(!path.exists());
}

#[test]
fn rm_multiple_files() {
    let (_dir, path1) = temp_file_with("content1\n");
    let (_dir2, path2) = temp_file_with("content2\n");

    rb(&["rm", path1.to_str().unwrap(), path2.to_str().unwrap()])
        .assert()
        .success();

    assert!(!path1.exists());
    assert!(!path2.exists());
}

#[test]
fn rm_recursive_directory() {
    let dir = temp_dir();
    let file = dir.path().join("a.txt");
    fs::write(&file, "content\n").unwrap();

    rb(&["rm", "-r", dir.path().to_str().unwrap()])
        .assert()
        .success();

    assert!(!dir.path().exists());
}

#[test]
fn rm_force_ignore_nonexistent() {
    rb(&["rm", "-f", "/nonexistent"]).assert().success();
}

#[test]
fn rm_force_ignore_nonexistent_with_other_files() {
    let (_dir, path) = temp_file_with("content\n");
    rb(&["rm", "-f", "/nonexistent", path.to_str().unwrap()])
        .assert()
        .success();

    assert!(!path.exists());
}

#[test]
fn rm_interactive_yes() {
    let (_dir, path) = temp_file_with("content\n");
    rb(&["rm", "-i", path.to_str().unwrap()])
        .write_stdin("y\n")
        .assert()
        .success();

    assert!(!path.exists());
}

#[test]
fn rm_interactive_no() {
    let (_dir, path) = temp_file_with("content\n");
    rb(&["rm", "-i", path.to_str().unwrap()])
        .write_stdin("n\n")
        .assert()
        .success();

    assert!(path.exists());
}

#[test]
fn rm_verbose() {
    let (_dir, path) = temp_file_with("content\n");
    rb(&["rm", "-v", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("removed"));
}
// === Error cases ===

#[test]
fn rm_nonexistent_no_force() {
    rb(&["rm", "/nonexistent"]).assert().failure().code(1);
}

#[test]
fn rm_no_args() {
    rb(&["rm"]).assert().failure().code(1);
}

#[test]
fn rm_directory_without_recursive() {
    let dir = temp_dir();
    rb(&["rm", dir.path().to_str().unwrap()])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn rm_root_refused() {
    rb(&["rm", "-rf", "/"]).assert().failure().code(1);
}

#[test]
fn rm_remove_symlink_not_target() {
    let dir = temp_dir();
    let target = dir.path().join("target");
    fs::write(&target, "data\n").unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    rb(&["rm", link.to_str().unwrap()]).assert().success();
    assert!(!link.exists());
    assert!(target.exists());
}

#[test]
fn rm_recursive_with_hardlinks() {
    let dir = temp_dir();
    let file = dir.path().join("file");
    fs::write(&file, "data\n").unwrap();
    let hard = dir.path().join("hard");
    fs::hard_link(&file, &hard).unwrap();

    rb(&["rm", "-f", file.to_str().unwrap()]).assert().success();
    assert!(hard.exists());
}
