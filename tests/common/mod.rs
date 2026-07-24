// Copyright (c) 2026 Semyon Tsarev
// SPDX-License-Identifier: MPL-2.0

use assert_cmd::Command;
use std::path::PathBuf;
use tempfile::TempDir;

/// Return a `Command` pointing to the built `rustybox` binary.
pub fn rustybox() -> Command {
    Command::cargo_bin("rustybox").unwrap()
}

/// Shortcut: rustybox with pre-filled arguments.
pub fn rb(args: &[&str]) -> Command {
    let mut cmd = rustybox();
    cmd.args(args);
    cmd
}

/// Create a temporary directory whose handle will clean up on drop.
#[allow(dead_code)]
pub fn temp_dir() -> TempDir {
    tempfile::tempdir().unwrap()
}

/// Create a temporary file with `content` and return (handle, full path).
#[allow(dead_code)]
pub fn temp_file_with(content: &str) -> (TempDir, PathBuf) {
    let dir = temp_dir();
    let path = dir.path().join("test.txt");
    std::fs::write(&path, content).unwrap();
    (dir, path)
}

/// Create a small file tree for testing recursive commands.
#[allow(dead_code)]
pub fn create_file_tree() -> TempDir {
    let dir = temp_dir();
    std::fs::write(dir.path().join("a.txt"), "alpha\n").unwrap();
    std::fs::write(dir.path().join("b.txt"), "beta\ngamma\n").unwrap();
    let sub = dir.path().join("sub");
    std::fs::create_dir(&sub).unwrap();
    std::fs::write(sub.join("c.txt"), "delta\nepsilon\nzeta\n").unwrap();
    dir
}