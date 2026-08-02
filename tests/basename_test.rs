// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use rstest::rstest;

use common::rb;

// === POSIX basename behavior ===
// Specification: https://man.archlinux.org/man/basename.1p
// Steps:
// 1. If string is null → unspecified (either '.' or null string)
// 2. If string is "//" → implementation-defined
// 3. If string consists entirely of slashes → set to single '/'
// 4. Remove trailing slashes
// 5. Remove prefix up to and including last slash
// 6. Remove suffix if present, only if suffix matches and resulting string non-empty

#[test]
fn basename_simple_path() {
    rb(&["basename", "/usr/bin/ls"])
        .assert()
        .success()
        .stdout(predicate::eq("ls\n"));
}

#[test]
fn basename_path_with_trailing_slash() {
    rb(&["basename", "/usr/bin/"])
        .assert()
        .success()
        .stdout(predicate::eq("bin\n"));
}

#[test]
fn basename_no_directory() {
    rb(&["basename", "file.txt"])
        .assert()
        .success()
        .stdout(predicate::eq("file.txt\n"));
}

#[test]
fn basename_only_slashes() {
    // POSIX: if string consists entirely of slash characters → set to single '/'
    rb(&["basename", "///"])
        .assert()
        .success()
        .stdout(predicate::eq("/\n"));
}

#[test]
fn basename_root() {
    rb(&["basename", "/"])
        .assert()
        .success()
        .stdout(predicate::eq("/\n"));
}

#[test]
fn basename_empty_string() {
    // POSIX: unspecified whether result is '.' or null string
    // Most implementations (including this one) return empty string
    rb(&["basename", ""])
        .assert()
        .success()
        .stdout(predicate::eq("\n"));
}

#[rstest]
#[case("/usr/bin/ls", "ls")]
#[case("file.txt", "file.txt")]
#[case("/", "/")]
#[case("///", "/")]
#[case("a/b/c/", "c")]
#[case("usr/", "usr")]
fn basename_various_paths(#[case] path: &str, #[case] expected: &str) {
    rb(&["basename", path])
        .assert()
        .success()
        .stdout(predicate::eq(format!("{}\n", expected)));
}

// === Suffix removal tests ===

#[test]
fn basename_suffix_removal() {
    rb(&["basename", "/usr/bin/ls", "s"])
        .assert()
        .success()
        .stdout(predicate::eq("l\n"));
}

#[test]
fn basename_suffix_removal_full_match() {
    // POSIX: suffix removal only if remaining string is non-empty
    rb(&["basename", "file.txt", ".txt"])
        .assert()
        .success()
        .stdout(predicate::eq("file\n"));
}

#[test]
fn basename_suffix_not_found() {
    // Not an error if suffix not found
    rb(&["basename", "file.txt", ".log"])
        .assert()
        .success()
        .stdout(predicate::eq("file.txt\n"));
}

#[test]
fn basename_suffix_removes_entire_string() {
    // POSIX: suffix removal only if remaining string is non-empty
    // "file" with suffix "file" → should return "file" (not empty)
    rb(&["basename", "file", "file"])
        .assert()
        .success()
        .stdout(predicate::eq("file\n"));
}

#[test]
fn basename_suffix_with_trailing_slashes() {
    rb(&["basename", "/usr/bin/", "in"])
        .assert()
        .success()
        .stdout(predicate::eq("b\n"));
}

#[test]
fn basename_double_slash() {
    // POSIX: implementation-defined for "//"
    // Most implementations return "/"
    rb(&["basename", "//"])
        .assert()
        .success()
        .stdout(predicate::eq("/\n"));
}

// === Error cases ===

#[test]
fn basename_too_many_args() {
    // POSIX: only string and optional suffix are supported
    rb(&["basename", "/a/b", "b", "extra"])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn basename_suffix_removal_with_trailing_slash() {
    rb(&["basename", "/usr/bin/", "bin"])
        .assert()
        .success()
        .stdout(predicate::eq("bin\n"));
}

#[test]
fn basename_suffix_does_not_match_full_string() {
    rb(&["basename", "foo", "foo"])
        .assert()
        .success()
        .stdout(predicate::eq("foo\n"));
}

#[test]
fn basename_suffix_empty() {
    rb(&["basename", "file.txt", ""])
        .assert()
        .success()
        .stdout(predicate::eq("file.txt\n"));
}

#[test]
fn basename_multiple_slashes_after_path() {
    rb(&["basename", "a//b//c/"])
        .assert()
        .success()
        .stdout(predicate::eq("c\n"));
}
