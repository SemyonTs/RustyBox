// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use rstest::rstest;

use common::rb;

// === POSIX xargs behavior ===
// Specification: https://manpages.opensuse.org/Leap-16.0/man-pages-posix/xargs.1p.en.html
// - Build and execute command lines from stdin
// - -n number: Max arguments per command
// - -I repl: Replace string in initial arguments
// - -0: NUL-separated input

#[test]
fn xargs_default_echo() {
    rb(&["xargs"])
        .write_stdin("a b c\n")
        .assert()
        .success()
        .stdout(predicate::eq("a b c\n"));
}

#[test]
fn xargs_with_command() {
    rb(&["xargs", "echo", "prefix"])
        .write_stdin("a b\n")
        .assert()
        .success()
        .stdout(predicate::eq("prefix a b\n"));
}

#[test]
fn xargs_n_max_args() {
    rb(&["xargs", "-n", "2", "echo"])
        .write_stdin("a b c d\n")
        .assert()
        .success()
        .stdout(predicate::eq("a b\nc d\n"));
}

#[test]
fn xargs_I_replace() {
    rb(&["xargs", "-I", "{}", "echo", "prefix", "{}", "suffix"])
        .write_stdin("a\nb\n")
        .assert()
        .success()
        .stdout(predicate::eq("prefix a suffix\nprefix b suffix\n"));
}

#[test]
fn xargs_0_nul_separated() {
    rb(&["xargs", "-0", "echo"])
        .write_stdin(b"a\0b\0c\0")
        .assert()
        .success()
        .stdout(predicate::eq("a b c\n"));
}

#[test]
fn xargs_empty_input() {
    rb(&["xargs", "echo"])
        .write_stdin("")
        .assert()
        .success()
        .stdout(predicate::eq(""));
}

// === Error cases ===

#[test]
fn xargs_invalid_n() {
    rb(&["xargs", "-n", "0", "echo"])
        .write_stdin("a\n")
        .assert()
        .failure()
        .code(1);
}

#[test]
fn xargs_command_not_found() {
    rb(&["xargs", "nonexistent"])
        .write_stdin("a\n")
        .assert()
        .failure()
        .code(1);
}

#[test]
fn xargs_L_max_lines() {
    rb(&["xargs", "-L", "2", "echo"])
        .write_stdin("a\nb\nc\nd\n")
        .assert()
        .success()
        .stdout(predicate::eq("a b\nc d\n"));
}

#[test]
fn xargs_P_max_processes() {
    rb(&["xargs", "-P", "2", "echo"])
        .write_stdin("a\nb\n")
        .assert()
        .success()
        .stdout(predicate::eq("a b\n"));
}

#[test]
fn xargs_empty_input_no_command() {
    rb(&["xargs", "echo"])
        .write_stdin("")
        .assert()
        .success()
        .stdout(predicate::eq(""));
}

#[test]
fn xargs_I_with_empty_replacement() {
    rb(&["xargs", "-I", "{}", "echo", "{}"])
        .write_stdin("a\nb\n")
        .assert()
        .success()
        .stdout(predicate::eq("a\nb\n"));
}
