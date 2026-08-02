// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use rstest::rstest;
use std::fs;
use std::os::unix::fs::PermissionsExt;

use common::{rb, temp_dir, temp_file_with};

// === POSIX chmod behavior ===
// Specification: https://manpages.ubuntu.com/manpages/xenial/en/man1/chmod.1posix.html
// - Change file mode bits as specified by mode operand
// - -R: Recursively change directories and their contents
// - Symbolic mode: [who] op perm [, ...]
// - who: u, g, o, a
// - op: +, -, =
// - perm: r, w, x, s, t

// === Octal mode tests ===

#[test]
fn chmod_octal_absolute() {
    let (_dir, path) = temp_file_with("test\n");
    rb(&["chmod", "755", path.to_str().unwrap()])
        .assert()
        .success();

    let meta = fs::metadata(&path).unwrap();
    assert_eq!(meta.permissions().mode() & 0o7777, 0o755);
}

#[test]
fn chmod_octal_sticky_bit() {
    let (_dir, path) = temp_file_with("test\n");
    rb(&["chmod", "1755", path.to_str().unwrap()])
        .assert()
        .success();

    let meta = fs::metadata(&path).unwrap();
    assert_eq!(meta.permissions().mode() & 0o7777, 0o1755);
}

#[test]
fn chmod_octal_setuid() {
    let (_dir, path) = temp_file_with("test\n");
    rb(&["chmod", "4755", path.to_str().unwrap()])
        .assert()
        .success();

    let meta = fs::metadata(&path).unwrap();
    assert_eq!(meta.permissions().mode() & 0o7777, 0o4755);
}

// === Symbolic mode tests ===

#[test]
fn chmod_symbolic_add() {
    let (_dir, path) = temp_file_with("test\n");
    // Start with 600 (rw-------)
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

    rb(&["chmod", "g+rx", path.to_str().unwrap()])
        .assert()
        .success();

    let meta = fs::metadata(&path).unwrap();
    // Should be 650 (rw-r-x---)
    assert_eq!(meta.permissions().mode() & 0o7777, 0o650);
}

#[test]
fn chmod_symbolic_remove() {
    let (_dir, path) = temp_file_with("test\n");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();

    rb(&["chmod", "o-w", path.to_str().unwrap()])
        .assert()
        .success();

    let meta = fs::metadata(&path).unwrap();
    assert_eq!(meta.permissions().mode() & 0o7777, 0o755);
}

#[test]
fn chmod_symbolic_assign() {
    let (_dir, path) = temp_file_with("test\n");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o777)).unwrap();

    rb(&["chmod", "u=rw,go=r", path.to_str().unwrap()])
        .assert()
        .success();

    let meta = fs::metadata(&path).unwrap();
    // Should be 644 (rw-r--r--)
    assert_eq!(meta.permissions().mode() & 0o7777, 0o644);
}

#[test]
fn chmod_symbolic_multiple_clauses() {
    let (_dir, path) = temp_file_with("test\n");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

    rb(&["chmod", "u+x,g+w,o+r", path.to_str().unwrap()])
        .assert()
        .success();

    let meta = fs::metadata(&path).unwrap();
    // 0o600 + u+x (0100) + g+w (0020) + o+r (0004) = 0o724 (rwx-w-r--)
    assert_eq!(meta.permissions().mode() & 0o7777, 0o724);
}

#[test]
fn chmod_symbolic_a_all() {
    let (_dir, path) = temp_file_with("test\n");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

    rb(&["chmod", "a+x", path.to_str().unwrap()])
        .assert()
        .success();

    let meta = fs::metadata(&path).unwrap();
    // Should be 755 (rwxr-xr-x)
    assert_eq!(meta.permissions().mode() & 0o7777, 0o755);
}

#[test]
fn chmod_symbolic_setuid() {
    let (_dir, path) = temp_file_with("test\n");
    rb(&["chmod", "u+s", path.to_str().unwrap()])
        .assert()
        .success();

    let meta = fs::metadata(&path).unwrap();
    assert!(meta.permissions().mode() & 0o4000 != 0);
}

// === -R recursive tests ===

#[test]
fn chmod_recursive() {
    let dir = temp_dir();
    let file1 = dir.path().join("a.txt");
    let sub = dir.path().join("sub");
    let file2 = sub.join("b.txt");

    fs::write(&file1, "a\n").unwrap();
    fs::create_dir(&sub).unwrap();
    fs::write(&file2, "b\n").unwrap();

    rb(&["chmod", "-R", "700", dir.path().to_str().unwrap()])
        .assert()
        .success();

    let meta1 = fs::metadata(&file1).unwrap();
    let meta2 = fs::metadata(&file2).unwrap();
    assert_eq!(meta1.permissions().mode() & 0o7777, 0o700);
    assert_eq!(meta2.permissions().mode() & 0o7777, 0o700);
}

// === -v verbose and -c change-only tests ===

#[test]
fn chmod_verbose() {
    let (_dir, path) = temp_file_with("test\n");
    rb(&["chmod", "-v", "755", path.to_str().unwrap()])
        .assert()
        .success()
        .stderr(
            predicate::str::contains("mode of")
                .and(predicate::str::contains(path.to_str().unwrap())),
        );
}

#[test]
fn chmod_change_only_no_change() {
    let (_dir, path) = temp_file_with("test\n");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();

    rb(&["chmod", "-c", "755", path.to_str().unwrap()])
        .assert()
        .success()
        .stderr(predicate::eq("")); // No output when no change
}

#[test]
fn chmod_change_only_with_change() {
    let (_dir, path) = temp_file_with("test\n");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

    rb(&["chmod", "-c", "755", path.to_str().unwrap()])
        .assert()
        .success()
        .stderr(predicate::str::contains("changed from 0644"));
}

// === Error cases ===

#[test]
fn chmod_nonexistent_file() {
    rb(&["chmod", "755", "/nonexistent"])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn chmod_invalid_mode() {
    let (_dir, path) = temp_file_with("test\n");
    rb(&["chmod", "invalid", path.to_str().unwrap()])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn chmod_no_args() {
    rb(&["chmod"]).assert().failure().code(1);
}

#[test]
fn chmod_symbolic_setgid() {
    let (_dir, path) = temp_file_with("test\n");
    rb(&["chmod", "g+s", path.to_str().unwrap()])
        .assert()
        .success();
    let meta = fs::metadata(&path).unwrap();
    assert!(meta.permissions().mode() & 0o2000 != 0);
}

#[test]
fn chmod_symbolic_sticky_bit() {
    let (_dir, path) = temp_file_with("test\n");
    rb(&["chmod", "+t", path.to_str().unwrap()])
        .assert()
        .success();
    let meta = fs::metadata(&path).unwrap();
    assert!(meta.permissions().mode() & 0o1000 != 0);
}

#[test]
fn chmod_symbolic_multiple_who() {
    let (_dir, path) = temp_file_with("test\n");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    rb(&["chmod", "u=rwx,g=rx,o=r", path.to_str().unwrap()])
        .assert()
        .success();
    let meta = fs::metadata(&path).unwrap();
    assert_eq!(meta.permissions().mode() & 0o7777, 0o754);
}

#[test]
fn chmod_recursive_with_symbolic_mode() {
    let dir = temp_dir();
    let file = dir.path().join("a.txt");
    fs::write(&file, "test\n").unwrap();
    rb(&["chmod", "-R", "u+x", dir.path().to_str().unwrap()])
        .assert()
        .success();
    let meta = fs::metadata(&file).unwrap();
    assert!(meta.permissions().mode() & 0o100 != 0);
}
