// =============================================================================
// test — Evaluate conditional expressions.
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Implementation inspired by Toybox (https://landley.net/toybox/)
// Toybox is copyrighted by Rob Landley, see NOTICE file for license details.
//
// Supported operators:
//   Unary:  -e, -f, -d, -L, -h, -r, -w, -x, -s, -z, -n, -c, -b, -p, -S,
//           -u, -g, -k, -O, -G.
//   Binary: =, !=, -eq, -ne, -lt, -gt, -le, -ge, -nt, -ot, -ef.
//   Logical: !, -a, -o, ( ).
//
// Also implements the `[` synonym, which requires a trailing `]`.
// NOTE: test/[ does NOT use standard option parsing. All arguments are
// positional tokens evaluated by a recursive-descent parser per POSIX.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::fs;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// Entry point for `test` / `[`.
///
/// Unlike most builtins, test does not use args::parse because its syntax
/// does not conform to POSIX Utility Syntax Guidelines. All argv elements
/// after argv[0] are treated as positional expression tokens.
fn test_main(ctx: &mut Context) -> u8 {
    // Determine if we were invoked as "[" by checking argv[0] basename.
    let argv0_base = ctx.argv[0].rsplit('/').next().unwrap_or(&ctx.argv[0]);
    let is_bracket = argv0_base == "[";

    // Skip argv[0] (command name); everything else is expression tokens.
    let args: Vec<&str> = ctx.argv[1..].iter().map(|s| s.as_str()).collect();

    if is_bracket {
        // POSIX: when invoked as "[", the last argument must be "]".
        if args.is_empty() || *args.last().unwrap() != "]" {
            eprintln!("[: missing ']'");
            return 2;
        }
        // Evaluate everything except the trailing "]".
        let expr_args = &args[..args.len() - 1];
        if eval_expr(expr_args) { 0 } else { 1 }
    } else {
        if eval_expr(&args) { 0 } else { 1 }
    }
}

/// Evaluate a complete expression from a list of tokens.
fn eval_expr(args: &[&str]) -> bool {
    if args.is_empty() {
        return false;
    }
    let mut parser = Parser { args, pos: 0 };
    let result = parser.parse_or();
    // Per POSIX, if there are unconsumed tokens, results are unspecified.
    // We treat leftover tokens as an error condition returning false.
    if parser.pos < args.len() {
        return false;
    }
    result
}

/// Recursive-descent parser for `test` expressions.
struct Parser<'a> {
    args: &'a [&'a str],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&'a str> {
        self.args.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<&'a str> {
        let r = self.args.get(self.pos).copied();
        if r.is_some() {
            self.pos += 1;
        }
        r
    }

    /// `-o` (logical OR), lowest precedence.
    fn parse_or(&mut self) -> bool {
        let mut left = self.parse_and();
        while self.peek() == Some("-o") {
            self.next();
            let right = self.parse_and();
            left = left || right;
        }
        left
    }

    /// `-a` (logical AND), higher precedence than -o.
    fn parse_and(&mut self) -> bool {
        let mut left = self.parse_primary();
        while self.peek() == Some("-a") {
            self.next();
            let right = self.parse_primary();
            left = left && right;
        }
        left
    }

    /// Primary expressions: unary ops, binary ops, parens, negation, strings.
    fn parse_primary(&mut self) -> bool {
        // Parenthesised sub-expression.
        if self.peek() == Some("(") {
            self.next();
            let r = self.parse_or();
            if self.peek() == Some(")") {
                self.next();
            }
            return r;
        }

        // Negation.
        if self.peek() == Some("!") {
            self.next();
            return !self.parse_primary();
        }

        // Try unary operator: -X ARG
        if let Some(tok) = self.peek() {
            if let Some(op) = tok.strip_prefix('-') {
                if op.len() == 1 && is_unary(op) {
                    self.next(); // consume operator
                    if let Some(arg) = self.next() {
                        return eval_unary(op, arg);
                    }
                    // Unary op without argument: false
                    return false;
                }
            }
        }

        // Try binary operator: ARG OP ARG
        // Look ahead to see if current token + next token form a binary expr.
        if self.pos + 2 <= self.args.len() {
            let a = self.args[self.pos];
            if self.pos + 1 < self.args.len() {
                let op = self.args[self.pos + 1];
                if is_binary(op) && self.pos + 2 < self.args.len() {
                    let b = self.args[self.pos + 2];
                    self.pos += 3;
                    return eval_binary(a, op, b);
                }
            }
        }

        // Fallback: bare string — true if non-empty.
        if let Some(s) = self.next() {
            return !s.is_empty();
        }

        false
    }
}

/// Return `true` when `op` names a recognised unary operator.
fn is_unary(op: &str) -> bool {
    matches!(
        op,
        "e" | "f"
            | "d"
            | "L"
            | "h"
            | "r"
            | "w"
            | "x"
            | "s"
            | "z"
            | "n"
            | "c"
            | "b"
            | "p"
            | "u"
            | "g"
            | "k"
            | "O"
            | "G"
            | "S"
    )
}

/// Return `true` when `op` names a recognised binary operator.
fn is_binary(op: &str) -> bool {
    matches!(
        op,
        "=" | "!=" | "-eq" | "-ne" | "-lt" | "-gt" | "-le" | "-ge" | "-nt" | "-ot" | "-ef"
    )
}

/// Evaluate a unary expression.
fn eval_unary(op: &str, arg: &str) -> bool {
    match op {
        "e" => Path::new(arg).exists(),
        "f" => fs::metadata(arg).map(|m| m.is_file()).unwrap_or(false),
        "d" => fs::metadata(arg).map(|m| m.is_dir()).unwrap_or(false),
        "L" | "h" => fs::symlink_metadata(arg)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false),
        "r" => fs::metadata(arg)
            .map(|m| m.mode() & 0o400 != 0)
            .unwrap_or(false),
        "w" => fs::metadata(arg)
            .map(|m| m.mode() & 0o200 != 0)
            .unwrap_or(false),
        "x" => fs::metadata(arg)
            .map(|m| m.mode() & 0o100 != 0)
            .unwrap_or(false),
        "s" => fs::metadata(arg).map(|m| m.size() > 0).unwrap_or(false),
        "z" => arg.is_empty(),
        "n" => !arg.is_empty(),
        "c" => fs::metadata(arg)
            .map(|m| m.file_type().is_char_device())
            .unwrap_or(false),
        "b" => fs::metadata(arg)
            .map(|m| m.file_type().is_block_device())
            .unwrap_or(false),
        "p" => fs::metadata(arg)
            .map(|m| m.file_type().is_fifo())
            .unwrap_or(false),
        "S" => fs::metadata(arg)
            .map(|m| m.file_type().is_socket())
            .unwrap_or(false),
        "u" => fs::metadata(arg)
            .map(|m| m.mode() & 0o4000 != 0)
            .unwrap_or(false),
        "g" => fs::metadata(arg)
            .map(|m| m.mode() & 0o2000 != 0)
            .unwrap_or(false),
        "O" => fs::metadata(arg)
            .map(|m| m.uid() == unsafe { libc::getuid() })
            .unwrap_or(false),
        "G" => fs::metadata(arg)
            .map(|m| m.gid() == unsafe { libc::getgid() })
            .unwrap_or(false),
        "k" => fs::metadata(arg)
            .map(|m| m.mode() & 0o1000 != 0)
            .unwrap_or(false),
        _ => false,
    }
}

/// Evaluate a binary expression.
fn eval_binary(a: &str, op: &str, b: &str) -> bool {
    match op {
        "=" => a == b,
        "!=" => a != b,
        "-eq" => a.parse::<i64>().ok() == b.parse::<i64>().ok(),
        "-ne" => a.parse::<i64>().ok() != b.parse::<i64>().ok(),
        "-lt" => a.parse::<i64>().unwrap_or(0) < b.parse::<i64>().unwrap_or(0),
        "-gt" => a.parse::<i64>().unwrap_or(0) > b.parse::<i64>().unwrap_or(0),
        "-le" => a.parse::<i64>().unwrap_or(0) <= b.parse::<i64>().unwrap_or(0),
        "-ge" => a.parse::<i64>().unwrap_or(0) >= b.parse::<i64>().unwrap_or(0),
        "-nt" => {
            let ma = fs::metadata(a);
            let mb = fs::metadata(b);
            match (ma, mb) {
                (Ok(x), Ok(y)) => x.mtime() > y.mtime(),
                _ => false,
            }
        }
        "-ot" => {
            let ma = fs::metadata(a);
            let mb = fs::metadata(b);
            match (ma, mb) {
                (Ok(x), Ok(y)) => x.mtime() < y.mtime(),
                _ => false,
            }
        }
        "-ef" => {
            let ma = fs::metadata(a);
            let mb = fs::metadata(b);
            match (ma, mb) {
                (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
                _ => false,
            }
        }
        _ => false,
    }
}

register_command!(TEST_CMD, "test", "", CommandFlags::BIN.bits(), test_main);

register_command!(BRACKET_CMD, "[", "", CommandFlags::BIN.bits(), test_main);
