// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use rstest::rstest;
use std::os::unix::fs::MetadataExt;
use std::{fs, os::unix::fs::PermissionsExt};

use common::{rb, temp_dir, temp_file_with};

// === POSIX cp behavior ===
// Specification: https://man7.org/linux/man-pages/man1/cp.1p
// - Copy contents of source_file to target_file
// - -R: Copy directories recursively
// - -P: Copy symlinks as symlinks (default with -R)
// - -f: Force overwrite
// - -i: Interactive prompt before overwrite
// - -p: Preserve mode, ownership, timestamps

// === Basic copy tests ===

#[test]
fn cp_single_file() {
    let (_dir, src) = temp_file_with("hello\nworld\n");
    let dest = temp_dir().path().join("dest.txt");

    rb(&["cp", src.to_str().unwrap(), dest.to_str().unwrap()])
        .assert()
        .success();

    let content = fs::read_to_string(&dest).unwrap();
    assert_eq!(content, "hello\nworld\n");
}

#[test]
fn cp_into_directory() {
    let src_dir = temp_dir();
    let src = src_dir.path().join("file.txt");
    fs::write(&src, "content\n").unwrap();

    let dest_dir = temp_dir();
    rb(&[
        "cp",
        src.to_str().unwrap(),
        dest_dir.path().to_str().unwrap(),
    ])
    .assert()
    .success();

    let dest = dest_dir.path().join("file.txt");
    assert!(dest.exists());
    assert_eq!(fs::read_to_string(&dest).unwrap(), "content\n");
}

#[test]
fn cp_multiple_files_to_directory() {
    let src_dir1 = temp_dir();
    let src1 = src_dir1.path().join("a.txt");
    fs::write(&src1, "A\n").unwrap();

    let src_dir2 = temp_dir();
    let src2 = src_dir2.path().join("b.txt");
    fs::write(&src2, "B\n").unwrap();

    let dest_dir = temp_dir();
    rb(&[
        "cp",
        src1.to_str().unwrap(),
        src2.to_str().unwrap(),
        dest_dir.path().to_str().unwrap(),
    ])
    .assert()
    .success();

    assert!(dest_dir.path().join("a.txt").exists());
    assert!(dest_dir.path().join("b.txt").exists());
}

// === -R recursive copy tests ===
#[test]
fn cp_recursive_directory() {
    let src_dir = temp_dir();
    let sub = src_dir.path().join("sub");
    let file1 = src_dir.path().join("a.txt");
    let file2 = sub.join("b.txt");

    fs::write(&file1, "A\n").unwrap();
    fs::create_dir(&sub).unwrap();
    fs::write(&file2, "B\n").unwrap();

    let dest_dir = temp_dir();
    rb(&[
        "cp",
        "-R",
        src_dir.path().to_str().unwrap(),
        dest_dir.path().to_str().unwrap(),
    ])
    .assert()
    .success();

    // cp -R src_dir dest_dir copies the *directory* src_dir into dest_dir,
    // so the destination tree is dest_dir/<src_dir_name>/...
    let src_name = src_dir.path().file_name().unwrap();
    let dest_root = dest_dir.path().join(src_name);
    assert!(dest_root.exists());
    assert!(dest_root.join("a.txt").exists());
    assert!(dest_root.join("sub").exists());
    assert!(dest_root.join("sub/b.txt").exists());
}
// === -p preserve attributes tests ===

#[test]
fn cp_preserve_mode() {
    let (_dir, src) = temp_file_with("test\n");
    fs::set_permissions(&src, fs::Permissions::from_mode(0o755)).unwrap();

    let dest = temp_dir().path().join("dest.txt");
    rb(&["cp", "-p", src.to_str().unwrap(), dest.to_str().unwrap()])
        .assert()
        .success();

    let src_mode = fs::metadata(&src).unwrap().permissions().mode();
    let dest_mode = fs::metadata(&dest).unwrap().permissions().mode();
    assert_eq!(src_mode & 0o7777, dest_mode & 0o7777);
}

// === -f force overwrite tests ===

#[test]
fn cp_force_overwrite() {
    let (_dir, src) = temp_file_with("new\n");
    let (_dir2, dest) = temp_file_with("old\n");

    rb(&["cp", "-f", src.to_str().unwrap(), dest.to_str().unwrap()])
        .assert()
        .success();

    assert_eq!(fs::read_to_string(&dest).unwrap(), "new\n");
}

// === -i interactive prompt tests ===

#[test]
fn cp_interactive_overwrite_yes() {
    let (_dir, src) = temp_file_with("new\n");
    let (_dir2, dest) = temp_file_with("old\n");

    rb(&["cp", "-i", src.to_str().unwrap(), dest.to_str().unwrap()])
        .write_stdin("y\n")
        .assert()
        .success();

    assert_eq!(fs::read_to_string(&dest).unwrap(), "new\n");
}

#[test]
fn cp_interactive_overwrite_no() {
    let (_dir, src) = temp_file_with("new\n");
    let (_dir2, dest) = temp_file_with("old\n");
    let old_content = fs::read_to_string(&dest).unwrap();

    rb(&["cp", "-i", src.to_str().unwrap(), dest.to_str().unwrap()])
        .write_stdin("n\n")
        .assert()
        .success();

    assert_eq!(fs::read_to_string(&dest).unwrap(), old_content);
}

// === -n no-clobber tests ===

#[test]
fn cp_no_clobber() {
    let (_dir, src) = temp_file_with("new\n");
    let (_dir2, dest) = temp_file_with("old\n");
    let old_content = fs::read_to_string(&dest).unwrap();

    rb(&["cp", "-n", src.to_str().unwrap(), dest.to_str().unwrap()])
        .assert()
        .success();

    assert_eq!(fs::read_to_string(&dest).unwrap(), old_content);
}

// === -u update tests ===

#[test]
fn cp_update_newer_source() {
    let (_dir, src) = temp_file_with("new\n");
    let (_dir2, dest) = temp_file_with("old\n");

    // Make source newer
    let now = std::time::SystemTime::now();
    filetime::set_file_mtime(&src, filetime::FileTime::from_system_time(now)).unwrap();

    rb(&["cp", "-u", src.to_str().unwrap(), dest.to_str().unwrap()])
        .assert()
        .success();

    assert_eq!(fs::read_to_string(&dest).unwrap(), "new\n");
}

#[test]
fn cp_update_older_source() {
    let (_dir, src) = temp_file_with("old\n");
    let (_dir2, dest) = temp_file_with("new\n");

    // Make source older
    let old = std::time::SystemTime::UNIX_EPOCH;
    filetime::set_file_mtime(&src, filetime::FileTime::from_system_time(old)).unwrap();

    rb(&["cp", "-u", src.to_str().unwrap(), dest.to_str().unwrap()])
        .assert()
        .success();

    // Should not overwrite
    assert_eq!(fs::read_to_string(&dest).unwrap(), "new\n");
}

// === -s symbolic link tests ===

#[test]
fn cp_symbolic_link() {
    let (_dir, src) = temp_file_with("target\n");
    let dest = temp_dir().path().join("link");

    rb(&["cp", "-s", src.to_str().unwrap(), dest.to_str().unwrap()])
        .assert()
        .success();

    let link_target = fs::read_link(&dest).unwrap();
    assert_eq!(link_target, src);
}

// === -l hard link tests ===

#[test]
fn cp_hard_link() {
    let (_dir, src) = temp_file_with("content\n");
    let dest = temp_dir().path().join("hardlink");

    rb(&["cp", "-l", src.to_str().unwrap(), dest.to_str().unwrap()])
        .assert()
        .success();

    let src_ino = fs::metadata(&src).unwrap().ino();
    let dest_ino = fs::metadata(&dest).unwrap().ino();
    assert_eq!(src_ino, dest_ino);
}

// === Error cases ===

#[test]
fn cp_nonexistent_source() {
    let dest = temp_dir().path().join("dest.txt");
    rb(&["cp", "/nonexistent", dest.to_str().unwrap()])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn cp_missing_arguments() {
    rb(&["cp"]).assert().failure().code(1);
}

#[test]
fn cp_attempt_copy_directory_without_recursive() {
    let src_dir = temp_dir();
    let dest = temp_dir().path().join("dest");

    rb(&[
        "cp",
        src_dir.path().to_str().unwrap(),
        dest.to_str().unwrap(),
    ])
    .assert()
    .failure()
    .code(1);
}

#[test]
fn cp_preserve_symlinks_with_R() {
    let dir = temp_dir();
    let target = dir.path().join("target");
    fs::write(&target, "data\n").unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    let dest_dir = temp_dir();
    rb(&[
        "cp",
        "-R",
        link.to_str().unwrap(),
        dest_dir.path().to_str().unwrap(),
    ])
    .assert()
    .success();
    let copied_link = dest_dir.path().join("link");
    assert!(copied_link.is_symlink());
    assert_eq!(fs::read_link(&copied_link).unwrap(), target);
}

#[test]
fn cp_force_overrides_interactive() {
    let (_dir, src) = temp_file_with("new\n");
    let (_dir2, dest) = temp_file_with("old\n");
    rb(&[
        "cp",
        "-f",
        "-i",
        src.to_str().unwrap(),
        dest.to_str().unwrap(),
    ])
    .write_stdin("n\n")
    .assert()
    .success();
    assert_eq!(fs::read_to_string(&dest).unwrap(), "new\n");
}

#[test]
fn cp_no_clobber_with_existing_file() {
    let (_dir, src) = temp_file_with("new\n");
    let (_dir2, dest) = temp_file_with("old\n");
    rb(&["cp", "-n", src.to_str().unwrap(), dest.to_str().unwrap()])
        .assert()
        .success();
    assert_eq!(fs::read_to_string(&dest).unwrap(), "old\n");
}

#[test]
fn cp_update_source_newer_than_dest() {
    let (_dir, src) = temp_file_with("new\n");
    let (_dir2, dest) = temp_file_with("old\n");
    let now = std::time::SystemTime::now();
    filetime::set_file_mtime(&src, filetime::FileTime::from_system_time(now)).unwrap();
    rb(&["cp", "-u", src.to_str().unwrap(), dest.to_str().unwrap()])
        .assert()
        .success();
    assert_eq!(fs::read_to_string(&dest).unwrap(), "new\n");
}
