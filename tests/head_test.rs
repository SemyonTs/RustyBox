// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use rstest::rstest;

use common::{rb, temp_file_with};

#[test]
fn head_default_ten_lines() {
    let content = (1..=15).map(|i| format!("line {i}\n")).collect::<String>();
    let (_dir, path) = temp_file_with(&content);
    rb(&["head", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| out.lines().count() == 10));
}

#[rstest]
#[case(5)]
#[case(1)]
#[case(3)]
fn head_n_lines(#[case] n: usize) {
    let content = (1..=10).map(|i| format!("line {i}\n")).collect::<String>();
    let (_dir, path) = temp_file_with(&content);
    rb(&["head", "-n", &n.to_string(), path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::function(move |out: &str| {
            out.lines().count() == n
        }));
}

#[test]
fn head_stdin() {
    rb(&["head", "-n", "2"])
        .write_stdin("one\ntwo\nthree\nfour\n")
        .assert()
        .success()
        .stdout(predicate::eq("one\ntwo\n"));
}

#[test]
fn head_bytes() {
    let (_dir, path) = temp_file_with("hello world\n");
    rb(&["head", "-c", "5", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| out.len() == 5));
}

#[test]
fn head_quiet_mode() {
    let (_dir, path) = temp_file_with("test\n");
    rb(&["head", "-q", "-n", "1", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("test\n"));
}

// -v: always print header (single file)
#[test]
fn head_verbose_single() {
    let (_dir, path) = temp_file_with("line1\nline2\n");
    rb(&["head", "-v", "-n", "1", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq(format!(
            "==> {} <==\nline1\n",
            path.to_str().unwrap()
        )));
}

// -v with multiple files
#[test]
fn head_verbose_multiple() {
    let (_dir1, path1) = temp_file_with("a\n");
    let (_dir2, path2) = temp_file_with("b\n");
    rb(&[
        "head",
        "-v",
        "-n",
        "1",
        path1.to_str().unwrap(),
        path2.to_str().unwrap(),
    ])
    .assert()
    .success()
    .stdout(predicate::function(|out: &str| {
        out.contains("==> ")
            && out.contains(path1.to_str().unwrap())
            && out.contains("a")
            && out.contains(path2.to_str().unwrap())
            && out.contains("b")
    }));
}

// -q with multiple files (suppress headers)
#[test]
fn head_quiet_multiple() {
    let (_dir1, path1) = temp_file_with("a\n");
    let (_dir2, path2) = temp_file_with("b\n");
    rb(&[
        "head",
        "-q",
        "-n",
        "1",
        path1.to_str().unwrap(),
        path2.to_str().unwrap(),
    ])
    .assert()
    .success()
    .stdout(predicate::eq("a\nb\n"));
}

// -c 0 -> empty output
#[test]
fn head_bytes_zero() {
    let (_dir, path) = temp_file_with("hello\n");
    rb(&["head", "-c", "0", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq(""));
}

// -n 0 -> empty output
#[test]
fn head_lines_zero() {
    let (_dir, path) = temp_file_with("hello\n");
    rb(&["head", "-n", "0", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq(""));
}

// -c larger than file size -> entire file
#[test]
fn head_bytes_larger_than_file() {
    let (_dir, path) = temp_file_with("abc\n");
    rb(&["head", "-c", "10", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("abc\n"));
}

// Multiple files without -q or -v: headers are shown by default
#[test]
fn head_multiple_files_default_headers() {
    let (_dir1, path1) = temp_file_with("a\n");
    let (_dir2, path2) = temp_file_with("b\n");
    rb(&[
        "head",
        "-n",
        "1",
        path1.to_str().unwrap(),
        path2.to_str().unwrap(),
    ])
    .assert()
    .success()
    .stdout(predicate::function(|out: &str| {
        out.contains("==> ")
            && out.contains(path1.to_str().unwrap())
            && out.contains("a")
            && out.contains(path2.to_str().unwrap())
            && out.contains("b")
    }));
}

// Non‑existent file
#[test]
fn head_nonexistent_file() {
    rb(&["head", "/nonexistent"]).assert().failure().code(1);
}

// -c with stdin
#[test]
fn head_stdin_bytes() {
    rb(&["head", "-c", "5"])
        .write_stdin("hello world\n")
        .assert()
        .success()
        .stdout(predicate::eq("hello"));
}
