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
        .stdout(predicate::eq("yes\n"));
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

// -F: fixed string (literal) matching
#[test]
fn grep_fixed_strings() {
    let (_dir, path) = temp_file_with("hello\nworld\n");
    rb(&["grep", "-F", "hello", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("hello\n"));
}

// -w: whole word
#[test]
fn grep_whole_word() {
    let (_dir, path) = temp_file_with("test\ntesting\n");
    rb(&["grep", "-w", "test", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("test\n"));
}

// -x: whole line
#[test]
fn grep_whole_line() {
    let (_dir, path) = temp_file_with("test\nnot\ntest\n");
    rb(&["grep", "-x", "test", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("test\ntest\n"));
}

// -e: multiple patterns
#[test]
fn grep_multiple_patterns() {
    let (_dir, path) = temp_file_with("apple\nbanana\ncherry\n");
    rb(&[
        "grep",
        "-e",
        "apple",
        "-e",
        "cherry",
        path.to_str().unwrap(),
    ])
    .assert()
    .success()
    .stdout(predicate::eq("apple\ncherry\n"));
}

// -f: read patterns from file
#[test]
fn grep_pattern_file() {
    let (_dir, pat_path) = temp_file_with("apple\ncherry\n");
    let (_dir2, file_path) = temp_file_with("apple\nbanana\ncherry\n");
    rb(&[
        "grep",
        "-f",
        pat_path.to_str().unwrap(),
        file_path.to_str().unwrap(),
    ])
    .assert()
    .success()
    .stdout(predicate::eq("apple\ncherry\n"));
}

// -r: recursive search
#[test]
fn grep_recursive() {
    let dir = tempfile::tempdir().unwrap();
    let file1 = dir.path().join("a.txt");
    std::fs::write(&file1, "hello\nworld\n").unwrap();
    let subdir = dir.path().join("sub");
    std::fs::create_dir(&subdir).unwrap();
    let file2 = subdir.join("b.txt");
    std::fs::write(&file2, "hello again\n").unwrap();
    rb(&["grep", "-r", "hello", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| {
            out.contains("a.txt:hello") && out.contains("sub/b.txt:hello again")
        }));
}

// -l: list files with match
#[test]
fn grep_files_with_match() {
    let (_dir1, path1) = temp_file_with("match\n");
    let (_dir2, path2) = temp_file_with("no\n");
    rb(&[
        "grep",
        "-l",
        "match",
        path1.to_str().unwrap(),
        path2.to_str().unwrap(),
    ])
    .assert()
    .success()
    .stdout(predicate::eq(format!("{}\n", path1.to_str().unwrap())));
}

// -h: suppress filename
#[test]
fn grep_suppress_filename() {
    let (_dir, path) = temp_file_with("hello\nworld\n");
    rb(&["grep", "-h", "hello", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("hello\n"));
}

// -H: force filename
#[test]
fn grep_force_filename() {
    let (_dir, path) = temp_file_with("hello\n");
    rb(&["grep", "-H", "hello", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq(format!("{}:hello\n", path.to_str().unwrap())));
}

// Invalid regular expression
#[test]
fn grep_invalid_regex() {
    rb(&["grep", "[", "file"]).assert().failure().code(2);
}

// Non‑existent file
#[test]
fn grep_nonexistent_file() {
    rb(&["grep", "pattern", "/nonexistent"])
        .assert()
        .failure()
        .code(2);
}

// Multiple files – show filename by default
#[test]
fn grep_multiple_files() {
    let (_dir1, path1) = temp_file_with("one\n");
    let (_dir2, path2) = temp_file_with("two\n");
    rb(&[
        "grep",
        "one",
        path1.to_str().unwrap(),
        path2.to_str().unwrap(),
    ])
    .assert()
    .success()
    .stdout(predicate::eq(format!("{}:one\n", path1.to_str().unwrap())));
}
