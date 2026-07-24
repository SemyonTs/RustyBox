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
    rb(args)
        .assert()
        .success()
        .stdout(predicate::eq(expected));
}

#[test]
fn echo_no_args() {
    rb(&["echo"])
        .assert()
        .success()
        .stdout(predicate::eq("\n"));
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