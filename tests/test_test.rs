// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use rstest::rstest;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::fs::symlink;

use common::{rb, temp_dir, temp_file_with};

// === POSIX test behavior ===
// Specification: https://manpages.opensuse.org/Leap-16.0/man-pages-posix/test.1p.en.html
// - Evaluate conditional expressions
// - Unary: -e, -f, -d, -L, -h, -r, -w, -x, -s, -z, -n, -c, -b, -p, -S, -u, -g, -k, -O, -G
// - Binary: =, !=, -eq, -ne, -lt, -gt, -le, -ge, -nt, -ot, -ef
// - Logical: !, -a, -o, ( )
// - [ synonym requires trailing ]

// === File existence tests ===

#[test]
fn test_e_file_exists() {
    let (_dir, path) = temp_file_with("content\n");
    rb(&["test", "-e", path.to_str().unwrap()])
        .assert()
        .success()
        .code(0);
}

#[test]
fn test_e_file_not_exists() {
    rb(&["test", "-e", "/nonexistent"])
        .assert()
        .failure()
        .code(1);
}

// === File type tests ===

#[test]
fn test_f_regular_file() {
    let (_dir, path) = temp_file_with("content\n");
    rb(&["test", "-f", path.to_str().unwrap()])
        .assert()
        .success()
        .code(0);
}

#[test]
fn test_f_not_regular() {
    let dir = temp_dir();
    rb(&["test", "-f", dir.path().to_str().unwrap()])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn test_d_directory() {
    let dir = temp_dir();
    rb(&["test", "-d", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .code(0);
}

#[test]
fn test_d_not_directory() {
    let (_dir, path) = temp_file_with("content\n");
    rb(&["test", "-d", path.to_str().unwrap()])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn test_L_symlink() {
    let dir = temp_dir(); // Keep TempDir alive for the duration of the test
    let target = dir.path().join("target");
    fs::write(&target, "content\n").unwrap();

    let link = dir.path().join("link");
    symlink(&target, &link).unwrap();

    rb(&["test", "-L", link.to_str().unwrap()])
        .assert()
        .success()
        .code(0);
}

// === Permission tests ===

#[test]
fn test_r_readable() {
    let (_dir, path) = temp_file_with("content\n");
    rb(&["test", "-r", path.to_str().unwrap()])
        .assert()
        .success()
        .code(0);
}

#[test]
fn test_w_writable() {
    let (_dir, path) = temp_file_with("content\n");
    rb(&["test", "-w", path.to_str().unwrap()])
        .assert()
        .success()
        .code(0);
}

#[test]
fn test_x_executable() {
    let (_dir, path) = temp_file_with("content\n");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();

    rb(&["test", "-x", path.to_str().unwrap()])
        .assert()
        .success()
        .code(0);
}

// === Size tests ===

#[test]
fn test_s_non_empty() {
    let (_dir, path) = temp_file_with("content\n");
    rb(&["test", "-s", path.to_str().unwrap()])
        .assert()
        .success()
        .code(0);
}

#[test]
fn test_s_empty() {
    let (_dir, path) = temp_file_with("");
    rb(&["test", "-s", path.to_str().unwrap()])
        .assert()
        .failure()
        .code(1);
}

// === String tests ===

#[test]
fn test_z_empty_string() {
    rb(&["test", "-z", ""]).assert().success().code(0);
}

#[test]
fn test_z_non_empty() {
    rb(&["test", "-z", "hello"]).assert().failure().code(1);
}

#[test]
fn test_n_non_empty() {
    rb(&["test", "-n", "hello"]).assert().success().code(0);
}

// === Binary comparison tests ===

#[test]
fn test_equals() {
    rb(&["test", "a", "=", "a"]).assert().success().code(0);
}

#[test]
fn test_not_equals() {
    rb(&["test", "a", "!=", "b"]).assert().success().code(0);
}

#[test]
fn test_eq_numeric() {
    rb(&["test", "5", "-eq", "5"]).assert().success().code(0);
}

#[test]
fn test_lt_numeric() {
    rb(&["test", "3", "-lt", "5"]).assert().success().code(0);
}

#[test]
fn test_gt_numeric() {
    rb(&["test", "5", "-gt", "3"]).assert().success().code(0);
}

// === Logical operators ===

#[test]
fn test_not() {
    rb(&["test", "!", "-e", "/nonexistent"])
        .assert()
        .success()
        .code(0);
}

#[test]
fn test_and() {
    let (_dir, path) = temp_file_with("content\n");
    rb(&[
        "test",
        "-e",
        path.to_str().unwrap(),
        "-a",
        "-f",
        path.to_str().unwrap(),
    ])
    .assert()
    .success()
    .code(0);
}

#[test]
fn test_or() {
    rb(&["test", "-e", "/nonexistent", "-o", "-e", "/"])
        .assert()
        .success()
        .code(0);
}

// === [ synonym tests ===

#[test]
fn test_bracket_synonym() {
    let (_dir, path) = temp_file_with("content\n");
    rb(&["[", "-f", path.to_str().unwrap(), "]"])
        .assert()
        .success()
        .code(0);
}

#[test]
fn test_bracket_missing_closing() {
    let (_dir, path) = temp_file_with("content\n");
    rb(&["[", "-f", path.to_str().unwrap()])
        .assert()
        .failure()
        .code(2); // POSIX: error code 2 for missing ]
}

#[test]
fn test_nt_newer_than() {
    let dir = temp_dir();
    let file1 = dir.path().join("f1");
    let file2 = dir.path().join("f2");

    // Create file2 first with an old timestamp
    fs::write(&file2, "b\n").unwrap();
    let old_time = filetime::FileTime::from_unix_time(0, 0);
    filetime::set_file_mtime(&file2, old_time).unwrap();

    // Create file1 with current time (guaranteed newer)
    fs::write(&file1, "a\n").unwrap();
    let new_time = filetime::FileTime::now();
    filetime::set_file_mtime(&file1, new_time).unwrap();

    rb(&[
        "test",
        file1.to_str().unwrap(),
        "-nt",
        file2.to_str().unwrap(),
    ])
    .assert()
    .success()
    .code(0);
}

#[test]
fn test_ot_older_than() {
    let dir = temp_dir();
    let file1 = dir.path().join("f1");
    let file2 = dir.path().join("f2");

    // Create file1 with an old timestamp
    fs::write(&file1, "a\n").unwrap();
    let old_time = filetime::FileTime::from_unix_time(0, 0);
    filetime::set_file_mtime(&file1, old_time).unwrap();

    // Create file2 with current time (guaranteed newer)
    fs::write(&file2, "b\n").unwrap();
    let new_time = filetime::FileTime::now();
    filetime::set_file_mtime(&file2, new_time).unwrap();

    rb(&[
        "test",
        file1.to_str().unwrap(),
        "-ot",
        file2.to_str().unwrap(),
    ])
    .assert()
    .success()
    .code(0);
}

#[test]
fn test_ef_same_file() {
    let dir = temp_dir();
    let file = dir.path().join("f");
    fs::write(&file, "a\n").unwrap();
    let hard = dir.path().join("h");
    fs::hard_link(&file, &hard).unwrap();
    rb(&[
        "test",
        file.to_str().unwrap(),
        "-ef",
        hard.to_str().unwrap(),
    ])
    .assert()
    .success()
    .code(0);
}

#[test]
fn test_O_owned_by_user() {
    // Test against our own temp file which is guaranteed to be owned by current user.
    // Using /tmp is unreliable as it may be owned by root or have sticky bit issues.
    let (_dir, path) = temp_file_with("owned\n");
    rb(&["test", "-O", path.to_str().unwrap()])
        .assert()
        .success()
        .code(0);
}
