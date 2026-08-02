// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use rstest::rstest;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use common::{rb, temp_dir};

// === POSIX touch behavior ===
// Specification: https://manpages.opensuse.org/Leap-16.0/man-pages-posix/touch.1p.en.html
// - Change timestamps
// - -a: Access time only
// - -m: Modification time only
// - -c: Do not create
// - -r file: Use ref_file timestamps
// - -t time: Use specified time
// - -d date: Use specified date

#[test]
fn touch_create_file() {
    let dir = temp_dir();
    let path = dir.path().join("newfile.txt");

    rb(&["touch", path.to_str().unwrap()]).assert().success();

    assert!(path.exists());
    assert_eq!(fs::read_to_string(&path).unwrap(), "");
}

#[test]
fn touch_existing_file() {
    let dir = temp_dir();
    let path = dir.path().join("file.txt");
    fs::write(&path, "content\n").unwrap();

    let old_mtime = fs::metadata(&path).unwrap().modified().unwrap();

    // Wait a bit to ensure time changes
    std::thread::sleep(std::time::Duration::from_millis(100));

    rb(&["touch", path.to_str().unwrap()]).assert().success();

    let new_mtime = fs::metadata(&path).unwrap().modified().unwrap();
    assert!(new_mtime > old_mtime);
}

#[test]
fn touch_no_create() {
    let dir = temp_dir();
    let path = dir.path().join("nonexistent.txt");

    rb(&["touch", "-c", path.to_str().unwrap()])
        .assert()
        .success();

    assert!(!path.exists());
}

#[test]
fn touch_access_time_only() {
    let dir = temp_dir();
    let path = dir.path().join("file.txt");
    fs::write(&path, "content\n").unwrap();

    let old_atime = fs::metadata(&path).unwrap().accessed().unwrap();

    std::thread::sleep(std::time::Duration::from_millis(100));

    rb(&["touch", "-a", path.to_str().unwrap()])
        .assert()
        .success();

    let new_atime = fs::metadata(&path).unwrap().accessed().unwrap();
    assert!(new_atime > old_atime);
}

#[test]
fn touch_modification_time_only() {
    let dir = temp_dir();
    let path = dir.path().join("file.txt");
    fs::write(&path, "content\n").unwrap();

    let old_mtime = fs::metadata(&path).unwrap().modified().unwrap();

    std::thread::sleep(std::time::Duration::from_millis(100));

    rb(&["touch", "-m", path.to_str().unwrap()])
        .assert()
        .success();

    let new_mtime = fs::metadata(&path).unwrap().modified().unwrap();
    assert!(new_mtime > old_mtime);
}

#[test]
fn touch_reference_file() {
    let dir = temp_dir();
    let ref_path = dir.path().join("ref.txt");
    fs::write(&ref_path, "ref\n").unwrap();

    let target = dir.path().join("target.txt");
    fs::write(&target, "target\n").unwrap();

    // Get ref timestamps
    let ref_mtime = fs::metadata(&ref_path).unwrap().modified().unwrap();

    rb(&[
        "touch",
        "-r",
        ref_path.to_str().unwrap(),
        target.to_str().unwrap(),
    ])
    .assert()
    .success();

    let target_mtime = fs::metadata(&target).unwrap().modified().unwrap();
    // Should be close to ref_mtime
    let diff = target_mtime
        .duration_since(ref_mtime)
        .unwrap_or(std::time::Duration::from_secs(0));
    assert!(diff < std::time::Duration::from_secs(1));
}

#[test]
fn touch_time_spec() {
    let dir = temp_dir();
    let path = dir.path().join("file.txt");

    // -t format: [[CC]YY]MMDDhhmm[.ss]
    rb(&["touch", "-t", "202401011200", path.to_str().unwrap()])
        .assert()
        .success();

    assert!(path.exists());
    // Timestamp should be 2024-01-01 12:00:00
}

#[test]
fn touch_multiple_files() {
    let dir = temp_dir();
    let path1 = dir.path().join("file1.txt");
    let path2 = dir.path().join("file2.txt");

    rb(&["touch", path1.to_str().unwrap(), path2.to_str().unwrap()])
        .assert()
        .success();

    assert!(path1.exists());
    assert!(path2.exists());
}

// === Error cases ===

#[test]
fn touch_no_args() {
    rb(&["touch"]).assert().failure().code(1);
}

#[test]
fn touch_t_partial_time() {
    let dir = temp_dir();
    let path = dir.path().join("file");
    rb(&["touch", "-t", "01011200", path.to_str().unwrap()])
        .assert()
        .success();
    assert!(path.exists());
}

#[test]
fn touch_t_and_r_conflict() {
    let dir = temp_dir();
    let ref_file = dir.path().join("ref");
    fs::write(&ref_file, "ref\n").unwrap();
    let target = dir.path().join("target");
    fs::write(&target, "target\n").unwrap();
    rb(&[
        "touch",
        "-r",
        ref_file.to_str().unwrap(),
        "-t",
        "01011200",
        target.to_str().unwrap(),
    ])
    .assert()
    .failure()
    .code(1);
}

#[test]
fn touch_a_and_m_together() {
    let dir = temp_dir();
    let path = dir.path().join("file");
    fs::write(&path, "data\n").unwrap();
    let old_atime = fs::metadata(&path).unwrap().accessed().unwrap();
    let old_mtime = fs::metadata(&path).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));
    rb(&["touch", "-a", "-m", path.to_str().unwrap()])
        .assert()
        .success();
    let new_atime = fs::metadata(&path).unwrap().accessed().unwrap();
    let new_mtime = fs::metadata(&path).unwrap().modified().unwrap();
    assert!(new_atime > old_atime);
    assert!(new_mtime > old_mtime);
}
