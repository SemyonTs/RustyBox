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
            parts.len() == 3 && parts[0] == "2" // total lines
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

// -m: character count (multibyte aware)
#[test]
fn wc_characters() {
    let (_dir, path) = temp_file_with("hello\nworld\n");
    // "hello\nworld\n" has 12 characters (including two newlines)
    rb(&["wc", "-m", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("     12 \n"));
}

// Multiple files: total line at the end
#[test]
fn wc_multiple_files_total() {
    let (_dir1, path1) = temp_file_with("a\nb\n");
    let (_dir2, path2) = temp_file_with("c\nd\ne\n");
    rb(&["wc", path1.to_str().unwrap(), path2.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            let lines: Vec<&str> = out.lines().collect();
            lines.len() == 3 && lines[2].contains("total")
        }));
}

// Combination of options (-l and -w)
#[test]
fn wc_combination_options() {
    let (_dir, path) = temp_file_with("one two\nthree four\n");
    rb(&["wc", "-l", "-w", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("      2       4 \n"));
}

// Non‑existent file
#[test]
fn wc_nonexistent_file() {
    rb(&["wc", "/nonexistent"]).assert().failure().code(1);
}

// Empty file
#[test]
fn wc_empty_file() {
    let (_dir, path) = temp_file_with("");
    rb(&["wc", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("      0       0       0 \n"));
}
