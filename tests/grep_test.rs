// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use rstest::rstest;

use common::{rb, temp_file_with};

#[test]
fn grep_basic() {
    let (_dir, path) = temp_file_with("hello\nworld\nhello again\n");
    rb(&["grep", "hello", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("hello\nhello again\n"));
}

#[test]
fn grep_stdin() {
    rb(&["grep", "test"])
        .write_stdin("this is a test\nno match\nanother test\n")
        .assert()
        .success()
        .stdout(predicate::eq("this is a test\nanother test\n"));
}

#[test]
fn grep_invert_match() {
    let (_dir, path) = temp_file_with("match\nno match\nyes\n");
    rb(&["grep", "-v", "match", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("no match\nyes\n"));
}

#[test]
fn grep_case_insensitive() {
    let (_dir, path) = temp_file_with("Hello\nworld\n");
    rb(&["grep", "-i", "hello", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("Hello\n"));
}

#[test]
fn grep_count_only() {
    let (_dir, path) = temp_file_with("match\nmatch\nno\n");
    rb(&["grep", "-c", "match", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("2\n"));
}

#[test]
fn grep_line_numbers() {
    let (_dir, path) = temp_file_with("first\nsecond\nmatch\n");
    rb(&["grep", "-n", "match", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("3:match\n"));
}

#[test]
fn grep_quiet_mode() {
    let (_dir, path) = temp_file_with("test\n");
    rb(&["grep", "-q", "test", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq(""));
}

#[test]
fn grep_no_match() {
    let (_dir, path) = temp_file_with("nothing\nhere\n");
    rb(&["grep", "missing", path.to_str().unwrap()])
        .assert()
        .failure()
        .code(1);
}