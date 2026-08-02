// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use std::fs;

use common::{rb, temp_dir, temp_file_with};

// === POSIX link behavior ===
// Specification: https://manpages.ubuntu.com/manpages/xenial/en/man1/link.1posix.html
// - Perform link(file1, file2) function call
#[test]
fn link_creates_hard_link() {
    let (_dir, src) = temp_file_with("content\n");
    let dest_dir = temp_dir(); // Keep TempDir alive until the end of the test
    let dest = dest_dir.path().join("link");

    rb(&["link", src.to_str().unwrap(), dest.to_str().unwrap()])
        .assert()
        .success();

    assert!(dest.exists());
    assert_eq!(fs::read_to_string(&dest).unwrap(), "content\n");

    // Verify that this is a hard link (same inode)
    let src_metadata = fs::metadata(&src).unwrap();
    let dest_metadata = fs::metadata(&dest).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            src_metadata.ino(),
            dest_metadata.ino(),
            "Source and destination should have the same inode (hard link)"
        );
        assert_eq!(src_metadata.nlink(), 2, "Hard link count should be 2");
    }
}

#[test]
fn link_requires_two_arguments() {
    rb(&["link", "one"]).assert().failure().code(1);
}

#[test]
fn link_nonexistent_source() {
    let dest_dir = temp_dir(); // Keep TempDir alive
    let dest = dest_dir.path().join("link");
    rb(&["link", "/nonexistent", dest.to_str().unwrap()])
        .assert()
        .failure()
        .code(1);
}

// === POSIX unlink behavior ===
// Specification: https://manpages.ubuntu.com/manpages/xenial/en/man1/unlink.1posix.html
// - Perform unlink(file) function call

#[test]
fn unlink_removes_file() {
    let (_dir, path) = temp_file_with("content\n");
    rb(&["unlink", path.to_str().unwrap()]).assert().success();

    assert!(!path.exists());
}

#[test]
fn unlink_requires_one_argument() {
    rb(&["unlink"]).assert().failure().code(1);
}

#[test]
fn unlink_nonexistent_file() {
    rb(&["unlink", "/nonexistent"]).assert().failure().code(1);
}

#[test]
fn link_existing_dest_fails() {
    let (_dir, src) = temp_file_with("src\n");
    let (_dir2, dest) = temp_file_with("dest\n");
    rb(&["link", src.to_str().unwrap(), dest.to_str().unwrap()])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn unlink_directory_fails() {
    let dir = temp_dir();
    rb(&["unlink", dir.path().to_str().unwrap()])
        .assert()
        .failure()
        .code(1);
}
