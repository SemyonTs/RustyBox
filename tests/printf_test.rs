// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use rstest::rstest;

use common::rb;

// === POSIX printf behavior ===
// Specification: https://manpages.ubuntu.com/manpages/xenial/en/man1/printf.1posix.html

#[test]
fn printf_string() {
    rb(&["printf", "%s", "hello"])
        .assert()
        .success()
        .stdout(predicate::eq("hello"));
}

#[test]
fn printf_string_with_newline() {
    rb(&["printf", "%s\n", "hello"])
        .assert()
        .success()
        .stdout(predicate::eq("hello\n"));
}

#[test]
fn printf_integer_decimal() {
    rb(&["printf", "%d", "123"])
        .assert()
        .success()
        .stdout(predicate::eq("123"));
}

#[test]
fn printf_integer_hex() {
    rb(&["printf", "%x", "255"])
        .assert()
        .success()
        .stdout(predicate::eq("ff"));
}

#[test]
fn printf_integer_hex_upper() {
    rb(&["printf", "%X", "255"])
        .assert()
        .success()
        .stdout(predicate::eq("FF"));
}

#[test]
fn printf_integer_octal() {
    rb(&["printf", "%o", "255"])
        .assert()
        .success()
        .stdout(predicate::eq("377"));
}

#[test]
fn printf_float() {
    // POSIX/C standard: default precision for %f is 6
    rb(&["printf", "%f", "3.14"])
        .assert()
        .success()
        .stdout(predicate::eq("3.140000"));
}

#[test]
fn printf_float_precision() {
    rb(&["printf", "%.2f", "3.14159"])
        .assert()
        .success()
        .stdout(predicate::eq("3.14"));
}

#[test]
fn printf_character() {
    rb(&["printf", "%c", "abc"])
        .assert()
        .success()
        .stdout(predicate::eq("a"));
}

#[test]
fn printf_percent() {
    rb(&["printf", "%%"])
        .assert()
        .success()
        .stdout(predicate::eq("%"));
}

#[test]
fn printf_b_escape_newline() {
    rb(&["printf", "%b", "a\\nb"])
        .assert()
        .success()
        .stdout(predicate::eq("a\nb"));
}

#[test]
fn printf_b_escape_tab() {
    rb(&["printf", "%b", "a\\tb"])
        .assert()
        .success()
        .stdout(predicate::eq("a\tb"));
}

#[test]
fn printf_b_escape_backslash() {
    rb(&["printf", "%b", "a\\\\b"])
        .assert()
        .success()
        .stdout(predicate::eq("a\\b"));
}

#[test]
fn printf_multiple_arguments() {
    rb(&["printf", "%s %s", "hello", "world"])
        .assert()
        .success()
        .stdout(predicate::eq("hello world"));
}

#[test]
fn printf_field_width() {
    rb(&["printf", "%5d", "123"])
        .assert()
        .success()
        .stdout(predicate::eq("  123"));
}

#[test]
fn printf_precision_string() {
    rb(&["printf", "%.3s", "hello"])
        .assert()
        .success()
        .stdout(predicate::eq("hel"));
}

#[test]
fn printf_no_format() {
    rb(&["printf"]).assert().failure().code(1);
}

#[test]
fn printf_extra_arguments() {
    // POSIX: "The format operand shall be reused as often as necessary
    // to satisfy the argument operands."
    // Therefore, "%s" is reused for "world", resulting in "helloworld".
    rb(&["printf", "%s", "hello", "world"])
        .assert()
        .success()
        .stdout(predicate::eq("helloworld"));
}

#[test]
fn printf_float_scientific() {
    rb(&["printf", "%e", "3.14"])
        .assert()
        .success()
        .stdout(predicate::str::contains("e"));
}

#[test]
fn printf_missing_arguments() {
    // POSIX: Missing arguments for numeric conversions evaluate to 0.
    rb(&["printf", "%d %d", "1"])
        .assert()
        .success()
        .stdout(predicate::eq("1 0"));
}
