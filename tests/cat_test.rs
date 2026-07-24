// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use rstest::rstest;

use common::{rb, temp_file_with};

#[test]
fn cat_single_file() {
    let (_dir, path) = temp_file_with("hello\nworld\n");
    rb(&["cat", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("hello\nworld\n"));
}

#[test]
fn cat_stdin() {
    rb(&["cat"])
        .write_stdin("stdin test\n")
        .assert()
        .success()
        .stdout(predicate::eq("stdin test\n"));
}

#[test]
fn cat_multiple_files() {
    let (_dir1, path1) = temp_file_with("file1\n");
    let (_dir2, path2) = temp_file_with("file2\n");
    rb(&["cat", path1.to_str().unwrap(), path2.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("file1\nfile2\n"));
}

#[test]
fn cat_stdin_dash() {
    rb(&["cat", "-"])
        .write_stdin("stdin via dash\n")
        .assert()
        .success()
        .stdout(predicate::eq("stdin via dash\n"));
}

#[test]
fn cat_nonexistent_file() {
    rb(&["cat", "/nonexistent/file.txt"])
        .assert()
        .failure()
        .code(1);
}

#[rstest]
#[case(b"binary\x00\x01\x02\n")]
fn cat_binary_data(#[case] data: &[u8]) {
    rb(&["cat"])
        .write_stdin(data)
        .assert()
        .success()
        .stdout(predicate::eq(data));
}

#[test]
fn cat_visualize_newlines() {
    rb(&["cat", "-e"])
        .write_stdin("hello\n")
        .assert()
        .success()
        .stdout(predicate::eq("$\n"));
}

#[test]
fn cat_visualize_tabs() {
    rb(&["cat", "-t"])
        .write_stdin("\tindented\n")
        .assert()
        .success()
        .stdout(predicate::eq("^Iindented\n"));
}