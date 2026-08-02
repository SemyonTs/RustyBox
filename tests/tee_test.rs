// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use rstest::rstest;
use std::fs;

use common::{rb, temp_dir};

// === POSIX tee behavior ===
// Specification: https://manpages.ubuntu.com/manpages/xenial/en/man1/tee.1posix.html
// - Copy stdin to stdout and files
// - -a: Append to files
// - Should not buffer output

#[test]
fn tee_single_file() {
    let dir = temp_dir();
    let file = dir.path().join("out.txt");

    rb(&["tee", file.to_str().unwrap()])
        .write_stdin("hello\nworld\n")
        .assert()
        .success()
        .stdout(predicate::eq("hello\nworld\n"));

    assert_eq!(fs::read_to_string(&file).unwrap(), "hello\nworld\n");
}

#[test]
fn tee_multiple_files() {
    let dir = temp_dir();
    let file1 = dir.path().join("out1.txt");
    let file2 = dir.path().join("out2.txt");

    rb(&["tee", file1.to_str().unwrap(), file2.to_str().unwrap()])
        .write_stdin("test\n")
        .assert()
        .success();

    assert_eq!(fs::read_to_string(&file1).unwrap(), "test\n");
    assert_eq!(fs::read_to_string(&file2).unwrap(), "test\n");
}

#[test]
fn tee_append() {
    let dir = temp_dir();
    let file = dir.path().join("out.txt");
    fs::write(&file, "existing\n").unwrap();

    rb(&["tee", "-a", file.to_str().unwrap()])
        .write_stdin("new\n")
        .assert()
        .success();

    assert_eq!(fs::read_to_string(&file).unwrap(), "existing\nnew\n");
}

#[test]
fn tee_no_files() {
    // Should just copy stdin to stdout
    rb(&["tee"])
        .write_stdin("hello\n")
        .assert()
        .success()
        .stdout(predicate::eq("hello\n"));
}

#[test]
fn tee_binary_data() {
    let dir = temp_dir();
    let file = dir.path().join("out.bin");
    let data = b"binary\x00\x01\x02data\n";

    rb(&["tee", file.to_str().unwrap()])
        .write_stdin(data)
        .assert()
        .success();

    let content = fs::read(&file).unwrap();
    assert_eq!(&content, data);
}

#[test]
fn tee_with_existing_file_no_append_overwrites() {
    let dir = temp_dir();
    let file = dir.path().join("out.txt");
    fs::write(&file, "old\n").unwrap();
    rb(&["tee", file.to_str().unwrap()])
        .write_stdin("new\n")
        .assert()
        .success();
    assert_eq!(fs::read_to_string(&file).unwrap(), "new\n");
}

#[test]
fn tee_multiple_files_with_append() {
    let dir = temp_dir();
    let file1 = dir.path().join("out1.txt");
    let file2 = dir.path().join("out2.txt");
    fs::write(&file1, "old1\n").unwrap();
    fs::write(&file2, "old2\n").unwrap();
    rb(&[
        "tee",
        "-a",
        file1.to_str().unwrap(),
        file2.to_str().unwrap(),
    ])
    .write_stdin("new\n")
    .assert()
    .success();
    assert_eq!(fs::read_to_string(&file1).unwrap(), "old1\nnew\n");
    assert_eq!(fs::read_to_string(&file2).unwrap(), "old2\nnew\n");
}
