// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use rstest::rstest;
use std::fs;

use common::{rb, temp_dir};

// === POSIX rmdir behavior ===
// Specification: https://manpages.opensuse.org/Leap-16.0/man-pages-posix/rmdir.1p.en.html
// - Remove empty directories
// - -p: Remove parent directories as well

#[test]
fn rmdir_empty_directory() {
    let dir = temp_dir();
    let empty = dir.path().join("empty");
    fs::create_dir(&empty).unwrap();

    rb(&["rmdir", empty.to_str().unwrap()]).assert().success();

    assert!(!empty.exists());
}

#[test]
fn rmdir_multiple_directories() {
    let dir = temp_dir();
    let empty1 = dir.path().join("empty1");
    let empty2 = dir.path().join("empty2");
    fs::create_dir(&empty1).unwrap();
    fs::create_dir(&empty2).unwrap();

    rb(&["rmdir", empty1.to_str().unwrap(), empty2.to_str().unwrap()])
        .assert()
        .success();

    assert!(!empty1.exists());
    assert!(!empty2.exists());
}

#[test]
fn rmdir_p_parents() {
    let dir = temp_dir();
    let path = dir.path().join("a/b/c");
    fs::create_dir_all(&path).unwrap();

    rb(&["rmdir", "-p", path.to_str().unwrap()])
        .assert()
        .success();

    assert!(!dir.path().join("a").exists());
}

#[test]
fn rmdir_non_empty_directory() {
    let dir = temp_dir();
    let nonempty = dir.path().join("nonempty");
    fs::create_dir(&nonempty).unwrap();
    fs::write(nonempty.join("file.txt"), "content\n").unwrap();

    rb(&["rmdir", nonempty.to_str().unwrap()])
        .assert()
        .failure()
        .code(1);

    assert!(nonempty.exists());
}

// === Error cases ===

#[test]
fn rmdir_no_args() {
    rb(&["rmdir"]).assert().failure().code(1);
}

#[test]
fn rmdir_nonexistent() {
    rb(&["rmdir", "/nonexistent"]).assert().failure().code(1);
}

#[test]
fn rmdir_p_with_nonempty_parent() {
    let dir = temp_dir();
    let path = dir.path().join("a/b/c");
    fs::create_dir_all(&path).unwrap();
    fs::write(dir.path().join("a/file.txt"), "test\n").unwrap();

    rb(&["rmdir", "-p", path.to_str().unwrap()])
        .assert()
        .success();
    assert!(!path.exists());
    assert!(!dir.path().join("a/b").exists());
    assert!(dir.path().join("a").exists());
    assert!(dir.path().join("a/file.txt").exists());
}
