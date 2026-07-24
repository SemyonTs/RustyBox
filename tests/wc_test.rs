// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use rstest::rstest;

use common::{rb, temp_file_with};

#[test]
fn wc_default_counts() {
    let (_dir, path) = temp_file_with("hello world\ntest line\n");
    rb(&["wc", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            let parts: Vec<&str> = out.split_whitespace().collect();
            parts.len() == 3 && parts[2] == "2" // total lines
        }));
}

#[test]
fn wc_lines_only() {
    let (_dir, path) = temp_file_with("line1\nline2\nline3\n");
    rb(&["wc", "-l", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("      3 \n"));
}

#[test]
fn wc_words_only() {
    let (_dir, path) = temp_file_with("one two three\nfour\n");
    rb(&["wc", "-w", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("      4 \n"));
}

#[test]
fn wc_bytes_only() {
    let (_dir, path) = temp_file_with("hello\n");
    rb(&["wc", "-c", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("      6 \n"));
}

#[test]
fn wc_stdin() {
    rb(&["wc", "-w"])
        .write_stdin("a b c d\n")
        .assert()
        .success()
        .stdout(predicate::eq("      4 \n"));
}