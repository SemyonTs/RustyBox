// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use rstest::rstest;

use common::{rb, temp_file_with};

#[test]
fn tail_default_ten_lines() {
    let content = (1..=15).map(|i| format!("line {i}\n")).collect::<String>();
    let (_dir, path) = temp_file_with(&content);
    rb(&["tail", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            out.lines().count() == 10 && out.starts_with("line 6")
        }));
}

#[rstest]
#[case(5)]
#[case(1)]
#[case(3)]
fn tail_n_lines(#[case] n: usize) {
    let content = (1..=10).map(|i| format!("line {i}\n")).collect::<String>();
    let (_dir, path) = temp_file_with(&content);
    rb(&["tail", "-n", &n.to_string(), path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::function(move |out: &str| {
            out.lines().count() == n
        }));
}

#[test]
fn tail_stdin() {
    rb(&["tail", "-n", "2"])
        .write_stdin("one\ntwo\nthree\nfour\n")
        .assert()
        .success()
        .stdout(predicate::eq("three\nfour\n"));
}

#[test]
fn tail_bytes() {
    let (_dir, path) = temp_file_with("hello world\n");
    rb(&["tail", "-c", "6", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("world\n"));
}

#[test]
fn tail_from_line() {
    let content = "line1\nline2\nline3\nline4\nline5\n";
    let (_dir, path) = temp_file_with(content);
    rb(&["tail", "-n", "+3", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("line3\nline4\nline5\n"));
}

// -v: always print header (single file)
#[test]
fn tail_verbose_single() {
    let (_dir, path) = temp_file_with("line1\nline2\n");
    rb(&["tail", "-v", "-n", "1", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq(format!(
            "==> {} <==\nline2\n",
            path.to_str().unwrap()
        )));
}

// -v with multiple files
#[test]
fn tail_verbose_multiple() {
    let (_dir1, path1) = temp_file_with("a\n");
    let (_dir2, path2) = temp_file_with("b\n");
    rb(&[
        "tail",
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
fn tail_quiet_multiple() {
    let (_dir1, path1) = temp_file_with("a\n");
    let (_dir2, path2) = temp_file_with("b\n");
    rb(&[
        "tail",
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
fn tail_bytes_zero() {
    let (_dir, path) = temp_file_with("hello\n");
    rb(&["tail", "-c", "0", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq(""));
}

// -n 0 -> empty output
#[test]
fn tail_lines_zero() {
    let (_dir, path) = temp_file_with("hello\n");
    rb(&["tail", "-n", "0", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq(""));
}

// -c larger than file size -> entire file
#[test]
fn tail_bytes_larger_than_file() {
    let (_dir, path) = temp_file_with("abc\n");
    rb(&["tail", "-c", "10", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("abc\n"));
}

// -n +N with different values
#[test]
fn tail_from_line_plus_various() {
    let content = "line1\nline2\nline3\nline4\nline5\n";
    let (_dir, path) = temp_file_with(content);
    rb(&["tail", "-n", "+4", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("line4\nline5\n"));
}

// Multiple files without -q or -v: headers shown by default
#[test]
fn tail_multiple_files_default_headers() {
    let (_dir1, path1) = temp_file_with("a\n");
    let (_dir2, path2) = temp_file_with("b\n");
    rb(&[
        "tail",
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
fn tail_nonexistent_file() {
    rb(&["tail", "/nonexistent"]).assert().failure().code(1);
}
