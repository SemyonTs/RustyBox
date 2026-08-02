// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use rstest::rstest;

use common::{rb, temp_file_with};

// === POSIX sed behavior ===
// - Stream editor
// - s/pattern/replacement/[g]: Substitute
// - p: Print
// - d: Delete
// - a text: Append
// - i text: Insert
// - c text: Change
// - -n: Suppress automatic printing
// - -e script: Add script
// - -f file: Read script from file

#[test]
fn sed_substitute() {
    rb(&["sed", "s/foo/bar/"])
        .write_stdin("foo\n")
        .assert()
        .success()
        .stdout(predicate::eq("bar\n"));
}

#[test]
fn sed_substitute_global() {
    rb(&["sed", "s/foo/bar/g"])
        .write_stdin("foo foo\n")
        .assert()
        .success()
        .stdout(predicate::eq("bar bar\n"));
}

#[test]
fn sed_substitute_with_delimiter() {
    rb(&["sed", "s:foo:bar:"])
        .write_stdin("foo\n")
        .assert()
        .success()
        .stdout(predicate::eq("bar\n"));
}

#[test]
fn sed_print() {
    rb(&["sed", "-n", "p"])
        .write_stdin("hello\n")
        .assert()
        .success()
        .stdout(predicate::eq("hello\n"));
}

#[test]
fn sed_delete() {
    rb(&["sed", "/foo/d"])
        .write_stdin("foo\nbar\n")
        .assert()
        .success()
        .stdout(predicate::eq("bar\n"));
}

#[test]
fn sed_append() {
    rb(&["sed", "a appended"])
        .write_stdin("line\n")
        .assert()
        .success()
        .stdout(predicate::eq("line\nappended\n"));
}

#[test]
fn sed_insert() {
    rb(&["sed", "i inserted"])
        .write_stdin("line\n")
        .assert()
        .success()
        .stdout(predicate::eq("inserted\nline\n"));
}

#[test]
fn sed_change() {
    rb(&["sed", "c replaced"])
        .write_stdin("line\n")
        .assert()
        .success()
        .stdout(predicate::eq("replaced\n"));
}

#[test]
fn sed_multiple_commands() {
    rb(&["sed", "s/foo/bar/; s/baz/qux/"])
        .write_stdin("foo baz\n")
        .assert()
        .success()
        .stdout(predicate::eq("bar qux\n"));
}

#[test]
fn sed_e_script() {
    rb(&["sed", "-e", "s/foo/bar/"])
        .write_stdin("foo\n")
        .assert()
        .success()
        .stdout(predicate::eq("bar\n"));
}

#[test]
fn sed_f_script_file() {
    let (_dir, script_path) = temp_file_with("s/foo/bar/\n");
    rb(&["sed", "-f", script_path.to_str().unwrap()])
        .write_stdin("foo\n")
        .assert()
        .success()
        .stdout(predicate::eq("bar\n"));
}

#[test]
fn sed_n_suppress_default() {
    rb(&["sed", "-n", "s/foo/bar/"])
        .write_stdin("foo\n")
        .assert()
        .success()
        .stdout(predicate::eq("")); // No output without p
}

// === Error cases ===

#[test]
fn sed_no_script() {
    rb(&["sed"]).assert().failure().code(1);
}

#[test]
fn sed_invalid_script() {
    rb(&["sed", "invalid"])
        .write_stdin("foo\n")
        .assert()
        .failure()
        .code(1);
}

#[test]
fn sed_address_range() {
    rb(&["sed", "2,3s/foo/bar/"])
        .write_stdin("foo\nfoo\nfoo\nfoo\n")
        .assert()
        .success()
        .stdout(predicate::eq("foo\nbar\nbar\nfoo\n"));
}

#[test]
fn sed_substitute_with_backreference() {
    rb(&["sed", "s/foo\\(bar\\)/\\1/"])
        .write_stdin("foobar\n")
        .assert()
        .success()
        .stdout(predicate::eq("bar\n"));
}

#[test]
fn sed_delete_lines_matching_pattern() {
    rb(&["sed", "/^#/d"])
        .write_stdin("# comment\nline\n")
        .assert()
        .success()
        .stdout(predicate::eq("line\n"));
}

#[test]
fn sed_f_script_with_multiple_commands() {
    let (_dir, script_path) = temp_file_with("s/foo/bar/\ns/baz/qux/\n");
    rb(&["sed", "-f", script_path.to_str().unwrap()])
        .write_stdin("foo baz\n")
        .assert()
        .success()
        .stdout(predicate::eq("bar qux\n"));
}
