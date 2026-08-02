// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use rstest::rstest;

use common::{rb, temp_file_with};

// === POSIX cut behavior ===
// Specification: https://manpages.ubuntu.com/manpages/jammy/en/man1/cut.1posix.html
// - -b list: Cut bytes
// - -c list: Cut characters
// - -f list: Cut fields (delimited by -d, default TAB)
// - -d delim: Field delimiter
// - Lists: comma-separated or blank-separated, ranges: N, N-M, N-, -M

// === -b byte tests ===

#[test]
fn cut_bytes_single() {
    let (_dir, path) = temp_file_with("abcdef\n");
    rb(&["cut", "-b", "2", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("b\n"));
}

#[test]
fn cut_bytes_range() {
    let (_dir, path) = temp_file_with("abcdef\n");
    rb(&["cut", "-b", "2-4", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("bcd\n"));
}

#[test]
fn cut_bytes_open_range() {
    let (_dir, path) = temp_file_with("abcdef\n");
    rb(&["cut", "-b", "4-", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("def\n"));
}

#[test]
fn cut_bytes_from_start() {
    let (_dir, path) = temp_file_with("abcdef\n");
    rb(&["cut", "-b", "-3", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("abc\n"));
}

#[test]
fn cut_bytes_multiple_ranges() {
    let (_dir, path) = temp_file_with("abcdef\n");
    rb(&["cut", "-b", "1,3,5", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("ace\n"));
}

#[test]
fn cut_bytes_byte_not_present() {
    // Not an error to select bytes not present
    let (_dir, path) = temp_file_with("abc\n");
    rb(&["cut", "-b", "10", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("\n"));
}

// === -c character tests ===

#[test]
fn cut_characters_single() {
    let (_dir, path) = temp_file_with("abcde\n");
    rb(&["cut", "-c", "2", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("b\n"));
}

#[test]
fn cut_characters_unicode() {
    // Multi-byte characters
    let (_dir, path) = temp_file_with("aπb\n");
    rb(&["cut", "-c", "2", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("π\n"));
}

// === -f field tests ===

#[test]
fn cut_fields_default_delimiter() {
    // Default delimiter is TAB
    let (_dir, path) = temp_file_with("a\tb\tc\n");
    rb(&["cut", "-f", "2", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("b\n"));
}

#[test]
fn cut_fields_custom_delimiter() {
    let (_dir, path) = temp_file_with("a:b:c\n");
    rb(&["cut", "-d", ":", "-f", "2", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("b\n"));
}

#[test]
fn cut_fields_multiple() {
    let (_dir, path) = temp_file_with("a:b:c:d\n");
    rb(&["cut", "-d", ":", "-f", "1,3", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("a:c\n"));
}

#[test]
fn cut_fields_range() {
    let (_dir, path) = temp_file_with("a:b:c:d:e\n");
    rb(&["cut", "-d", ":", "-f", "2-4", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("b:c:d\n"));
}

#[test]
fn cut_fields_open_range() {
    let (_dir, path) = temp_file_with("a:b:c:d\n");
    rb(&["cut", "-d", ":", "-f", "2-", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("b:c:d\n"));
}

#[test]
fn cut_fields_no_delimiter() {
    // Lines with no field delimiters are passed through intact
    let (_dir, path) = temp_file_with("abc\n");
    rb(&["cut", "-f", "2", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("abc\n"));
}

// === Multiple files ===

#[test]
fn cut_multiple_files() {
    let (_dir1, path1) = temp_file_with("abc\n");
    let (_dir2, path2) = temp_file_with("def\n");

    rb(&[
        "cut",
        "-b",
        "2",
        path1.to_str().unwrap(),
        path2.to_str().unwrap(),
    ])
    .assert()
    .success()
    .stdout(predicate::eq("b\ne\n"));
}

// === Stdin ===

#[test]
fn cut_stdin() {
    rb(&["cut", "-b", "2"])
        .write_stdin("abc\n")
        .assert()
        .success()
        .stdout(predicate::eq("b\n"));
}

// === Error cases ===

#[test]
fn cut_no_mode_specified() {
    rb(&["cut"]).assert().failure().code(1);
}

#[test]
fn cut_invalid_range() {
    let (_dir, path) = temp_file_with("abc\n");
    rb(&["cut", "-b", "invalid", path.to_str().unwrap()])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn cut_nonexistent_file() {
    rb(&["cut", "-b", "1", "/nonexistent"])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn cut_characters_range_unicode() {
    let (_dir, path) = temp_file_with("αβγδ\n");
    rb(&["cut", "-c", "2-3", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("βγ\n"));
}

#[test]
fn cut_fields_with_custom_delimiter_multiple() {
    let (_dir, path) = temp_file_with("a:b:c:d\n");
    rb(&["cut", "-d", ":", "-f", "2,4", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("b:d\n"));
}

#[test]
fn cut_fields_empty_field() {
    let (_dir, path) = temp_file_with("a::c\n");
    rb(&["cut", "-d", ":", "-f", "2", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::eq("\n"));
}

#[test]
fn cut_bytes_range_invalid_start() {
    let (_dir, path) = temp_file_with("abc\n");
    rb(&["cut", "-b", "0-2", path.to_str().unwrap()])
        .assert()
        .failure()
        .code(1);
}
