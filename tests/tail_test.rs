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