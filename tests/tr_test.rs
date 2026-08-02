// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use rstest::rstest;

use common::rb;

// === POSIX tr behavior ===
// Specification: https://manpages.opensuse.org/Leap-16.0/man-pages-posix/tr.1p.en.html
// - Translate or delete characters
// - -d: Delete characters in set1
// - -s: Squeeze repeats
// - -c: Complement set

#[test]
fn tr_translate() {
    rb(&["tr", "abc", "123"])
        .write_stdin("abc")
        .assert()
        .success()
        .stdout(predicate::eq("123"));
}

#[test]
fn tr_translate_range() {
    rb(&["tr", "a-z", "A-Z"])
        .write_stdin("hello")
        .assert()
        .success()
        .stdout(predicate::eq("HELLO"));
}

#[test]
fn tr_delete() {
    rb(&["tr", "-d", "aeiou"])
        .write_stdin("hello world")
        .assert()
        .success()
        .stdout(predicate::eq("hll wrld"));
}

#[test]
fn tr_squeeze() {
    rb(&["tr", "-s", " "])
        .write_stdin("hello    world")
        .assert()
        .success()
        .stdout(predicate::eq("hello world"));
}

#[test]
fn tr_complement() {
    rb(&["tr", "-c", "a", "x"])
        .write_stdin("abc")
        .assert()
        .success()
        .stdout(predicate::eq("axx"));
}

#[test]
fn tr_escape_sequences() {
    rb(&["tr", "\\n", "\\t"])
        .write_stdin("a\nb")
        .assert()
        .success()
        .stdout(predicate::eq("a\tb"));
}

#[test]
fn tr_empty_input() {
    rb(&["tr", "a", "b"])
        .write_stdin("")
        .assert()
        .success()
        .stdout(predicate::eq(""));
}

// === Error cases ===

#[test]
fn tr_no_args() {
    rb(&["tr"]).assert().failure().code(1);
}

#[test]
fn tr_missing_set2_for_translation() {
    rb(&["tr", "a"]).assert().failure().code(1);
}

#[test]
fn tr_squeeze_all_repeats() {
    rb(&["tr", "-s", "a"])
        .write_stdin("aaabbb")
        .assert()
        .success()
        .stdout(predicate::eq("abbb"));
}

#[test]
fn tr_complement_with_range() {
    rb(&["tr", "-c", "a-z", "X"])
        .write_stdin("abc123")
        .assert()
        .success()
        .stdout(predicate::eq("abcXXX"));
}

#[test]
fn tr_escape_octal() {
    rb(&["tr", "\\101", "A"])
        .write_stdin("A")
        .assert()
        .success()
        .stdout(predicate::eq("A"));
}

#[test]
fn tr_empty_set2_for_delete() {
    rb(&["tr", "-d", "a"])
        .write_stdin("abc")
        .assert()
        .success()
        .stdout(predicate::eq("bc"));
}
