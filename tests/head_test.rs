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
        .stdout(predicate::function(|out: &str| {
            out.lines().count() == 10
        }));
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
        .stdout(predicate::function(|out: &str| {
            out.len() == 5
        }));
}

#[test]
fn head_quiet_mode() {
    let (_dir, path) = temp_file_with("test\n");
    rb(&["head", "-q", "-n", "1", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("test\n"));
}