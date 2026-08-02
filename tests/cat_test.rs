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
        .stdout(predicate::eq(data.to_vec()));
}

// Test -e: visualize line endings with '$' (also implies -v)
#[test]
fn cat_visualize_newlines() {
    // -e implies -v, so newline is shown as '$' and other chars are visualized
    rb(&["cat", "-e"])
        .write_stdin("hello\n")
        .assert()
        .success()
        .stdout(predicate::eq("hello$\n"));
}

// Test -t: visualize tabs as ^I (also implies -v)
#[test]
fn cat_visualize_tabs() {
    rb(&["cat", "-t"])
        .write_stdin("\tindented\n")
        .assert()
        .success()
        .stdout(predicate::eq("^Iindented\n"));
}

// Test -v: visualize non-printing characters (except tab and newline)
#[test]
fn cat_visualize_nonprinting() {
    let data = b"\x01\x02\x7f\n";
    rb(&["cat", "-v"])
        .write_stdin(data)
        .assert()
        .success()
        .stdout(predicate::eq(b"^A^B^?\n".to_vec()));
}

// Test -t combined with -v (implied) – tabs become ^I, others also visualized
#[test]
fn cat_visualize_tabs_and_nonprinting() {
    let data = b"\t\x01\n";
    rb(&["cat", "-t"])
        .write_stdin(data)
        .assert()
        .success()
        // -t implies -v, so tabs become ^I and \x01 becomes ^A
        .stdout(predicate::eq(b"^I^A\n".to_vec()));
}

// Test -v -t -e together
#[test]
fn cat_visualize_all() {
    let data = b"\t\x01\n";
    rb(&["cat", "-vte"])
        .write_stdin(data)
        .assert()
        .success()
        // -v -t -e: tabs -> ^I, \x01 -> ^A, newline -> $\n
        .stdout(predicate::eq(b"^I^A$\n".to_vec()));
}

// Test -u (unbuffered) with text
#[test]
fn cat_unbuffered() {
    rb(&["cat", "-u"])
        .write_stdin("hello\n")
        .assert()
        .success()
        .stdout(predicate::eq("hello\n"));
}

// Test -u with binary data
#[test]
fn cat_unbuffered_binary() {
    let data = b"binary\x00\x01\n";
    rb(&["cat", "-u"])
        .write_stdin(data)
        .assert()
        .success()
        .stdout(predicate::eq(data.to_vec()));
}

// Test "-" with options
#[test]
fn cat_stdin_dash_with_options() {
    rb(&["cat", "-v", "-"])
        .write_stdin("\x01\n")
        .assert()
        .success()
        .stdout(predicate::eq(b"^A\n".to_vec()));
}

#[test]
fn cat_multiple_files_with_stdin_dash() {
    let (_dir1, path1) = temp_file_with("file1\n");
    let (_dir2, path2) = temp_file_with("file2\n");
    rb(&["cat", path1.to_str().unwrap(), "-", path2.to_str().unwrap()])
        .write_stdin("stdin\n")
        .assert()
        .success()
        .stdout(predicate::eq("file1\nstdin\nfile2\n"));
}

#[test]
fn cat_visualize_tab_and_newline() {
    rb(&["cat", "-et"])
        .write_stdin("\t\n")
        .assert()
        .success()
        .stdout(predicate::eq("^I$\n"));
}

#[test]
fn cat_visualize_nonprinting_without_newline() {
    rb(&["cat", "-v"])
        .write_stdin(b"\x1b")
        .assert()
        .success()
        .stdout(predicate::eq(b"^[".to_vec()));
}

#[test]
fn cat_unbuffered_with_multiple_files() {
    let (_dir, path) = temp_file_with("hello\n");
    rb(&["cat", "-u", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("hello\n"));
}
