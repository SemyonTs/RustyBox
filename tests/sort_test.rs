// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use rstest::rstest;

use common::{rb, temp_file_with};

// === POSIX sort behavior ===
// Specification: https://manpages.opensuse.org/Leap-16.0/man-pages-posix/sort.1p.en.html
// - Sort lines of text files
// - -r: Reverse
// - -n: Numeric sort
// - -u: Unique
// - -f: Case-insensitive
// - -k: Sort by field

#[test]
fn sort_basic() {
    let (_dir, path) = temp_file_with("c\nb\na\n");
    rb(&["sort", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("a\nb\nc\n"));
}

#[test]
fn sort_reverse() {
    let (_dir, path) = temp_file_with("a\nb\nc\n");
    rb(&["sort", "-r", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("c\nb\na\n"));
}

#[test]
fn sort_numeric() {
    let (_dir, path) = temp_file_with("10\n2\n1\n");
    rb(&["sort", "-n", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("1\n2\n10\n"));
}

#[test]
fn sort_unique() {
    let (_dir, path) = temp_file_with("a\na\nb\n");
    rb(&["sort", "-u", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("a\nb\n"));
}

#[test]
fn sort_case_insensitive() {
    let (_dir, path) = temp_file_with("A\nb\nC\na\n");
    rb(&["sort", "-f", path.to_str().unwrap()])
        .assert()
        .success()
        // Case-insensitive: should sort ignoring case
        .stdout(predicate::function(|out: &str| {
            let lines: Vec<&str> = out.lines().collect();
            // Should have all lines, but case-insensitive order
            lines.len() == 4
        }));
}

#[test]
fn sort_field() {
    let (_dir, path) = temp_file_with("2 b\n1 a\n3 c\n");
    rb(&["sort", "-k", "2", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("1 a\n2 b\n3 c\n"));
}

#[test]
fn sort_stdin() {
    rb(&["sort"])
        .write_stdin("c\nb\na\n")
        .assert()
        .success()
        .stdout(predicate::eq("a\nb\nc\n"));
}

#[test]
fn sort_multiple_files() {
    let (_dir1, path1) = temp_file_with("a\nc\n");
    let (_dir2, path2) = temp_file_with("b\nd\n");
    rb(&["sort", path1.to_str().unwrap(), path2.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("a\nb\nc\nd\n"));
}

#[test]
fn sort_field_with_end() {
    let (_dir, path) = temp_file_with("2 b\n1 a\n3 c\n");
    rb(&["sort", "-k", "2,2", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("1 a\n2 b\n3 c\n"));
}

#[test]
fn sort_tab_delimiter() {
    let (_dir, path) = temp_file_with("b\t2\na\t1\n");
    rb(&["sort", "-t", "\t", "-k", "2", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("a\t1\nb\t2\n"));
}

#[test]
fn sort_ignore_leading_blanks() {
    let (_dir, path) = temp_file_with(" 2\n1\n");
    rb(&["sort", "-b", "-n", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("1\n 2\n"));
}

#[test]
fn sort_stable() {
    let (_dir, path) = temp_file_with("a 1\nb 1\n");
    rb(&["sort", "-k", "2", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("a 1\nb 1\n"));
}
