// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use rstest::rstest;

use common::rb;

// === POSIX echo behavior ===
// Specification: https://manpages.opensuse.org/Leap-16.0/man-pages-posix/echo.1p.en.html
// - Write arguments to stdout, followed by newline
// - Arguments separated by single space
// - -n: Suppress trailing newline
// - Escape sequences: \a, \b, \c, \f, \n, \r, \t, \v, \\
// - Note: POSIX echo behavior is implementation-defined for -n and escapes

#[rstest]
#[case(&["echo", "hello"], "hello\n")]
#[case(&["echo", "-n", "hello"], "hello")]
#[case(&["echo", "a", "b", "c"], "a b c\n")]
#[case(&["echo", ""], "\n")]
fn echo_basic(#[case] args: &[&str], #[case] expected: &str) {
    rb(args).assert().success().stdout(predicate::eq(expected));
}

// === Escape sequence tests ===

#[test]
fn echo_escape_newline() {
    rb(&["echo", "a\\nb"])
        .assert()
        .success()
        .stdout(predicate::eq("a\\nb\n"));
}

#[test]
fn echo_escape_tab() {
    rb(&["echo", "a\\tb"])
        .assert()
        .success()
        .stdout(predicate::eq("a\\tb\n"));
}

#[test]
fn echo_escape_backslash() {
    rb(&["echo", "a\\\\b"])
        .assert()
        .success()
        .stdout(predicate::eq("a\\\\b\n"));
}

#[test]
fn echo_escape_carriage_return() {
    rb(&["echo", "-e", "a\\rb"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| out.contains('\r')));
}

// === -n option tests ===

#[test]
fn echo_no_newline() {
    rb(&["echo", "-n", "hello", "world"])
        .assert()
        .success()
        .stdout(predicate::eq("hello world"));
}

#[test]
fn echo_no_newline_with_escapes() {
    rb(&["echo", "-n", "a\\nb"])
        .assert()
        .success()
        .stdout(predicate::eq("a\\nb"));
}

// === Multiple arguments tests ===

#[test]
fn echo_multiple_args() {
    rb(&["echo", "one", "two", "three"])
        .assert()
        .success()
        .stdout(predicate::eq("one two three\n"));
}

#[test]
fn echo_args_with_spaces() {
    rb(&["echo", "hello world", "goodbye"])
        .assert()
        .success()
        .stdout(predicate::eq("hello world goodbye\n"));
}

// === Error cases ===

#[test]
fn echo_no_args() {
    rb(&["echo"]).assert().success().stdout(predicate::eq("\n"));
}

#[test]
fn echo_n_not_first_argument() {
    rb(&["echo", "hello", "-n", "world"])
        .assert()
        .success()
        .stdout(predicate::eq("hello -n world\n"));
}

#[test]
fn echo_empty_argument() {
    rb(&["echo", ""])
        .assert()
        .success()
        .stdout(predicate::eq("\n"));
}
