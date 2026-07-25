// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use rstest::rstest;

use common::rb;

#[rstest]
#[case(&["echo", "hello"], "hello\n")]
#[case(&["echo", "-n", "hello"], "hello")]
#[case(&["echo", "a", "b", "c"], "a b c\n")]
#[case(&["echo", "-e", "a\\nb"], "a\nb\n")]
fn echo_basic(#[case] args: &[&str], #[case] expected: &str) {
    rb(args).assert().success().stdout(predicate::eq(expected));
}

#[test]
fn echo_no_args() {
    rb(&["echo"]).assert().success().stdout(predicate::eq("\n"));
}

#[rstest]
#[case("\\t", "\t")]
#[case("\\r", "\r")]
#[case("\\\\", "\\")]
fn echo_escapes(#[case] esc: &str, #[case] expected: &str) {
    rb(&["echo", "-e", &format!("a{}b", esc)])
        .assert()
        .success()
        .stdout(predicate::eq(format!("a{}b\n", expected)));
}

// Without -e, backslashes are printed literally
#[test]
fn echo_no_escape() {
    rb(&["echo", "a\\nb"])
        .assert()
        .success()
        .stdout(predicate::eq("a\\nb\n"));
}

// Multiple escapes in one argument
#[test]
fn echo_escape_multiple() {
    rb(&["echo", "-e", "a\\tb\\nc"])
        .assert()
        .success()
        .stdout(predicate::eq("a\tb\nc\n"));
}

// Carriage return – check that output contains \r
#[test]
fn echo_escape_carriage_return() {
    rb(&["echo", "-e", "a\\rb"])
        .assert()
        .success()
        .stdout(predicate::function(|out: &str| out.contains('\r')));
}

// -n and -e together
#[test]
fn echo_n_and_e() {
    rb(&["echo", "-n", "-e", "a\\nb"])
        .assert()
        .success()
        .stdout(predicate::eq("a\nb"));
}

// Unknown escape sequence is preserved as literal backslash + character
#[test]
fn echo_unknown_escape() {
    rb(&["echo", "-e", "a\\xb"])
        .assert()
        .success()
        .stdout(predicate::eq("a\\xb\n"));
}
