// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use rstest::rstest;
use std::fs;
use std::os::unix::fs::symlink;

use common::{rb, temp_dir, temp_file_with};

// === POSIX/GNU readlink behavior ===
// - Print target of symbolic link
// -f: Canonicalize (follow all symlinks, final component may be missing)
// -e: Canonicalize (fail if final component missing)
// -m: Canonicalize without touching filesystem

#[test]
fn readlink_simple_symlink() {
    let (_dir, target) = temp_file_with("content\n");
    let dir = temp_dir();
    let link = dir.path().join("link");
    symlink(&target, &link).unwrap();

    rb(&["readlink", link.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq(format!("{}\n", target.to_str().unwrap())));
}

#[test]
fn readlink_not_a_symlink() {
    let (_dir, file) = temp_file_with("content\n");
    rb(&["readlink", file.to_str().unwrap()])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn readlink_nonexistent() {
    rb(&["readlink", "/nonexistent"]).assert().failure().code(1);
}

// === -f canonicalize tests ===

#[test]
fn readlink_f_canonicalize() {
    let dir = temp_dir();
    let real = dir.path().join("real");
    fs::write(&real, "content\n").unwrap();

    let link1 = dir.path().join("link1");
    symlink(&real, &link1).unwrap();

    let link2 = dir.path().join("link2");
    symlink(&link1, &link2).unwrap();

    rb(&["readlink", "-f", link2.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            out.trim().contains("real") && out.trim().starts_with('/')
        }));
}

#[test]
fn readlink_f_missing_final_component() {
    let dir = temp_dir();
    let link = dir.path().join("link");
    symlink("/nonexistent", &link).unwrap();

    // GNU readlink -f resolves as much as possible and allows the final
    // component to be missing.
    // NOTE: This test exposes a bug if rustybox uses std::fs::canonicalize()
    // internally, as it requires all path components to exist. The
    // implementation in readlink.rs needs a custom canonicalization logic
    // to pass this test.
    rb(&["readlink", "-f", link.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            out.trim().contains("nonexistent")
        }));
}

// === -e canonicalize (fail on missing) tests ===

#[test]
fn readlink_e_missing_final_component() {
    let dir = temp_dir();
    let link = dir.path().join("link");
    symlink("/nonexistent", &link).unwrap();

    rb(&["readlink", "-e", link.to_str().unwrap()])
        .assert()
        .failure()
        .code(1);
}

// === -m lexical canonicalize tests ===

#[test]
fn readlink_m_lexical() {
    rb(&["readlink", "-m", "/a/b/../c/./d"])
        .assert()
        .success()
        .stdout(predicate::eq("/a/c/d\n"));
}

#[test]
fn readlink_m_with_symlink() {
    let dir = temp_dir();
    let real = dir.path().join("real");
    fs::write(&real, "content\n").unwrap();

    let link = dir.path().join("link");
    symlink(&real, &link).unwrap();

    // -m should not follow symlinks
    rb(&["readlink", "-m", link.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| out.trim().contains("link")));
}

// === -n no newline tests ===

#[test]
fn readlink_n_no_newline() {
    let (_dir, target) = temp_file_with("content\n");
    let dir = temp_dir();
    let link = dir.path().join("link");
    symlink(&target, &link).unwrap();

    let output = rb(&["readlink", "-n", link.to_str().unwrap()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8(output).unwrap();
    assert!(!stdout.ends_with('\n'));
}

// === -v verbose tests ===

#[test]
fn readlink_v_verbose() {
    let (_dir, target) = temp_file_with("content\n");
    let dir = temp_dir();
    let link = dir.path().join("link");
    symlink(&target, &link).unwrap();

    rb(&["readlink", "-v", link.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            out.contains(link.to_str().unwrap()) && out.contains(target.to_str().unwrap())
        }));
}

// === Multiple arguments test ===

#[test]
fn readlink_multiple_args() {
    // GNU readlink supports multiple arguments and processes them sequentially.
    let (_dir, target) = temp_file_with("data\n");
    let dir1 = temp_dir();
    let link1 = dir1.path().join("l1");
    let dir2 = temp_dir();
    let link2 = dir2.path().join("l2");

    std::os::unix::fs::symlink(&target, &link1).unwrap();
    std::os::unix::fs::symlink(&target, &link2).unwrap();

    let expected = format!(
        "{}\n{}\n",
        target.to_str().unwrap(),
        target.to_str().unwrap()
    );

    rb(&["readlink", link1.to_str().unwrap(), link2.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq(expected));
}
