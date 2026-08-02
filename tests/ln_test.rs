// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

mod common;

use predicates::prelude::*;
use std::fs;
use std::os::unix::fs::{MetadataExt, symlink};

use common::{rb, temp_dir, temp_file_with};

// === POSIX ln behavior ===
// Specification: https://manpages.opensuse.org/Leap-16.0/man-pages-posix/ln.1p.en.html
// - Create links between files
// - -s: Symbolic links
// - -f: Force overwrite
// - -n: Treat symlink to directory as file

#[test]
fn ln_hard_link() {
    let (_dir, src) = temp_file_with("content\n");
    let dest_dir = temp_dir(); // Keep TempDir alive
    let dest = dest_dir.path().join("link");

    rb(&["ln", src.to_str().unwrap(), dest.to_str().unwrap()])
        .assert()
        .success();

    assert!(dest.exists());
    assert_eq!(fs::read_to_string(&dest).unwrap(), "content\n");

    // Verify that this is a hard link (same inode)
    let src_metadata = fs::metadata(&src).unwrap();
    let dest_metadata = fs::metadata(&dest).unwrap();

    assert_eq!(
        src_metadata.ino(),
        dest_metadata.ino(),
        "Source and destination should have the same inode (hard link)"
    );
    assert_eq!(src_metadata.nlink(), 2, "Hard link count should be 2");
}

#[test]
fn ln_symbolic_link() {
    let (_dir, src) = temp_file_with("content\n");
    let dest_dir = temp_dir(); // Keep TempDir alive
    let dest = dest_dir.path().join("link");

    rb(&["ln", "-s", src.to_str().unwrap(), dest.to_str().unwrap()])
        .assert()
        .success();

    assert!(dest.exists());
    assert_eq!(fs::read_link(&dest).unwrap(), src);
}

#[test]
fn ln_force_overwrite() {
    let (_dir, src) = temp_file_with("new\n");
    let (_dir2, dest) = temp_file_with("old\n");

    rb(&["ln", "-f", src.to_str().unwrap(), dest.to_str().unwrap()])
        .assert()
        .success();

    assert_eq!(fs::read_to_string(&dest).unwrap(), "new\n");
}

#[test]
fn ln_multiple_sources_to_directory() {
    let src_dir1 = temp_dir();
    let src1 = src_dir1.path().join("a.txt");
    fs::write(&src1, "A\n").unwrap();

    let src_dir2 = temp_dir();
    let src2 = src_dir2.path().join("b.txt");
    fs::write(&src2, "B\n").unwrap();

    let dest_dir = temp_dir();
    rb(&[
        "ln",
        src1.to_str().unwrap(),
        src2.to_str().unwrap(),
        dest_dir.path().to_str().unwrap(),
    ])
    .assert()
    .success();

    assert!(dest_dir.path().join("a.txt").exists());
    assert!(dest_dir.path().join("b.txt").exists());
}

// === -r relative link tests ===

#[test]
fn ln_relative_symbolic_link() {
    let dir = temp_dir();
    let src = dir.path().join("a.txt");
    fs::write(&src, "content\n").unwrap();

    let sub = dir.path().join("sub");
    fs::create_dir(&sub).unwrap();

    // Get the absolute path of the source.
    // A correct `ln -r` implementation should convert this absolute path
    // into a relative path ("../a.txt") based on the destination's directory.
    let absolute_src = src.canonicalize().unwrap();

    rb(&[
        "ln",
        "-s",
        "-r",
        absolute_src.to_str().unwrap(),
        sub.join("link").to_str().unwrap(),
    ])
    .assert()
    .success();

    let target = fs::read_link(sub.join("link")).unwrap();
    assert_eq!(target.to_str().unwrap(), "../a.txt");
}

// === -n no-dereference tests ===

#[test]
fn ln_no_dereference() {
    let dir = temp_dir();
    let target = dir.path().join("target");
    fs::write(&target, "target\n").unwrap();

    // Create a symlink 'link' that points to the directory 'dir'
    let link = dir.path().join("link");
    symlink(dir.path(), &link).unwrap();

    // -n ensures we overwrite the symlink 'link' itself,
    // not create 'target' inside the directory 'link' points to.
    // -f ensures we are allowed to overwrite.
    // -s ensures we create a symbolic link (so we can verify it with read_link).
    rb(&[
        "ln",
        "-s",
        "-n",
        "-f",
        target.to_str().unwrap(),
        link.to_str().unwrap(),
    ])
    .assert()
    .success();

    // Now 'link' should be a new symlink pointing to 'target'
    assert_eq!(fs::read_link(&link).unwrap(), target);
}

// === -L dereference tests ===

#[test]
fn ln_hard_link_to_symlink_default() {
    let dir = temp_dir();
    let target = dir.path().join("target");
    fs::write(&target, "data\n").unwrap();
    let sym = dir.path().join("sym");
    symlink(&target, &sym).unwrap();

    let hard = dir.path().join("hard");
    rb(&["ln", sym.to_str().unwrap(), hard.to_str().unwrap()])
        .assert()
        .success();

    // By default, ln creates a hard link to the symlink itself (like -P).
    // We MUST use symlink_metadata to get the inode of the symlink.
    assert_eq!(
        fs::symlink_metadata(&hard).unwrap().ino(),
        fs::symlink_metadata(&sym).unwrap().ino(),
        "Hard link should point to the symlink itself, not its target"
    );
}

#[test]
// NOTE: This test will fail until the `-L` flag is implemented in `src/commands/ln.rs`.
// If you haven't implemented `-L` yet, you can temporarily add `#[ignore]` above `#[test]`.
fn ln_hard_link_to_symlink_target_with_L() {
    let dir = temp_dir();
    let target = dir.path().join("target");
    fs::write(&target, "data\n").unwrap();
    let sym = dir.path().join("sym");
    symlink(&target, &sym).unwrap();

    let hard = dir.path().join("hard");
    rb(&["ln", "-L", sym.to_str().unwrap(), hard.to_str().unwrap()])
        .assert()
        .success();

    // With -L, ln dereferences the symlink and creates a hard link to the target.
    assert_eq!(
        fs::metadata(&hard).unwrap().ino(),
        fs::metadata(&target).unwrap().ino(),
        "Hard link should point to the target file"
    );
}
