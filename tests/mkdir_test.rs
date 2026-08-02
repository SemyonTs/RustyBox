// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use rstest::rstest;
use std::fs;
use std::os::unix::fs::PermissionsExt;

use common::{rb, temp_dir};

// === POSIX mkdir behavior ===
// Specification: https://manpages.ubuntu.com/manpages/xenial/en/man1/mkdir.1posix.html
// - Create directories
// - -p: Create intermediate directories
// - -m mode: Set mode

#[test]
fn mkdir_single_directory() {
    let dir = temp_dir();
    let new_dir = dir.path().join("newdir");

    rb(&["mkdir", new_dir.to_str().unwrap()]).assert().success();

    assert!(new_dir.exists());
    assert!(new_dir.is_dir());
}

#[test]
fn mkdir_multiple_directories() {
    let dir = temp_dir();
    let new_dir1 = dir.path().join("dir1");
    let new_dir2 = dir.path().join("dir2");

    rb(&[
        "mkdir",
        new_dir1.to_str().unwrap(),
        new_dir2.to_str().unwrap(),
    ])
    .assert()
    .success();

    assert!(new_dir1.exists());
    assert!(new_dir2.exists());
}

#[test]
fn mkdir_p_create_parents() {
    let dir = temp_dir();
    let path = dir.path().join("a/b/c");

    rb(&["mkdir", "-p", path.to_str().unwrap()])
        .assert()
        .success();

    assert!(path.exists());
}

#[test]
fn mkdir_p_existing_directory() {
    let dir = temp_dir();
    let existing = dir.path().join("existing");
    fs::create_dir(&existing).unwrap();

    // Should not fail if directory already exists
    rb(&["mkdir", "-p", existing.to_str().unwrap()])
        .assert()
        .success();
}

#[test]
fn mkdir_m_mode() {
    let dir = temp_dir();
    let new_dir = dir.path().join("newdir");

    rb(&["mkdir", "-m", "700", new_dir.to_str().unwrap()])
        .assert()
        .success();

    let meta = fs::metadata(&new_dir).unwrap();
    assert_eq!(meta.permissions().mode() & 0o7777, 0o700);
}

#[test]
fn mkdir_m_symbolic_mode() {
    let dir = temp_dir();
    let new_dir = dir.path().join("newdir");

    rb(&["mkdir", "-m", "u=rwx,go=rx", new_dir.to_str().unwrap()])
        .assert()
        .success();

    let meta = fs::metadata(&new_dir).unwrap();
    assert_eq!(meta.permissions().mode() & 0o7777, 0o755);
}

// === Error cases ===

#[test]
fn mkdir_no_args() {
    rb(&["mkdir"]).assert().failure().code(1);
}

#[test]
fn mkdir_existing_directory_no_p() {
    let dir = temp_dir();
    let existing = dir.path().join("existing");
    fs::create_dir(&existing).unwrap();

    rb(&["mkdir", existing.to_str().unwrap()])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn mkdir_p_with_mode_for_all_parents() {
    let dir = temp_dir();
    let path = dir.path().join("a/b/c");
    rb(&["mkdir", "-p", "-m", "700", path.to_str().unwrap()])
        .assert()
        .success();
    let meta_a = fs::metadata(dir.path().join("a")).unwrap();
    let meta_b = fs::metadata(dir.path().join("a/b")).unwrap();
    let meta_c = fs::metadata(&path).unwrap();
    assert_eq!(meta_a.permissions().mode() & 0o7777, 0o700);
    assert_eq!(meta_b.permissions().mode() & 0o7777, 0o700);
    assert_eq!(meta_c.permissions().mode() & 0o7777, 0o700);
}

#[test]
fn mkdir_p_file_exists_instead_of_dir() {
    let dir = temp_dir();
    let file = dir.path().join("a");
    fs::write(&file, "test\n").unwrap();
    let path = file.join("b");
    rb(&["mkdir", "-p", path.to_str().unwrap()])
        .assert()
        .failure()
        .code(1);
}
