// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use std::fs;

use common::{rb, temp_dir, temp_file_with};

// === POSIX mv behavior ===
// Specification: https://pubs.opengroup.org/onlinepubs/9699919799/utilities/mv.html

#[test]
fn mv_rename_file() {
    let (_dir, src) = temp_file_with("content\n");
    let dest_dir = temp_dir(); // Keep TempDir alive!
    let dest = dest_dir.path().join("dest.txt");

    rb(&["mv", src.to_str().unwrap(), dest.to_str().unwrap()])
        .assert()
        .success();

    assert!(!src.exists(), "Source file should no longer exist");
    assert!(dest.exists(), "Destination file should exist");
    assert_eq!(fs::read_to_string(&dest).unwrap(), "content\n");
}

#[test]
fn mv_move_to_existing_directory() {
    let (_dir, src) = temp_file_with("content\n");
    let dest_dir = temp_dir(); // Keep TempDir alive!

    rb(&[
        "mv",
        src.to_str().unwrap(),
        dest_dir.path().to_str().unwrap(),
    ])
    .assert()
    .success();

    let expected_dest = dest_dir.path().join("test.txt"); // temp_file_with uses "test.txt" as default name usually, or we check base name
    // Note: adjust "test.txt" if your temp_file_with uses a different default name.
    // Assuming it uses the filename from the temp path.
    assert!(dest_dir.path().join(src.file_name().unwrap()).exists());
}

#[test]
fn mv_force_overwrite() {
    let (_dir, src) = temp_file_with("new content\n");
    let dest_dir = temp_dir(); // Keep TempDir alive!
    let dest = dest_dir.path().join("dest.txt");

    // Create existing destination
    fs::write(&dest, "old content\n").unwrap();

    rb(&["mv", "-f", src.to_str().unwrap(), dest.to_str().unwrap()])
        .assert()
        .success();

    assert_eq!(fs::read_to_string(&dest).unwrap(), "new content\n");
}

#[test]
fn mv_no_clobber() {
    let (_dir, src) = temp_file_with("new content\n");
    let dest_dir = temp_dir(); // Keep TempDir alive!
    let dest = dest_dir.path().join("dest.txt");

    fs::write(&dest, "old content\n").unwrap();

    // -n should prevent overwrite and succeed (exit 0)
    rb(&["mv", "-n", src.to_str().unwrap(), dest.to_str().unwrap()])
        .assert()
        .success();

    assert_eq!(fs::read_to_string(&dest).unwrap(), "old content\n");
    assert!(src.exists(), "Source file should still exist");
}

#[test]
fn mv_directory_to_directory() {
    let src_dir = temp_dir(); // Keep alive
    let src_sub = src_dir.path().join("subdir");
    fs::create_dir(&src_sub).unwrap();
    fs::write(src_sub.join("file.txt"), "data\n").unwrap();

    let dest_dir = temp_dir(); // Keep alive!
    let dest_sub = dest_dir.path().join("new_subdir");

    rb(&["mv", src_sub.to_str().unwrap(), dest_sub.to_str().unwrap()])
        .assert()
        .success();

    assert!(!src_sub.exists());
    assert!(dest_sub.join("file.txt").exists());
    assert_eq!(
        fs::read_to_string(dest_sub.join("file.txt")).unwrap(),
        "data\n"
    );
}

#[test]
fn mv_nonexistent_source_fails() {
    let dest_dir = temp_dir(); // Keep alive
    let dest = dest_dir.path().join("dest.txt");

    rb(&["mv", "/nonexistent_file_12345", dest.to_str().unwrap()])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn mv_requires_arguments() {
    rb(&["mv"]).assert().failure().code(1);
    rb(&["mv", "only_one_arg"]).assert().failure().code(1);
}
