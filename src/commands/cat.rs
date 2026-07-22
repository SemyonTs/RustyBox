// =============================================================================
// cat — Concatenate and print files to standard output.
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Implementation inspired by Toybox (https://landley.net/toybox/)
// Toybox is copyrighted by Rob Landley, see NOTICE file for license details.
//
// Supported options:
//   -u      Unbuffered output (read/write 1 byte at a time).
//   -v      Visualize non-printing characters (except tab, newline).
//   -t      Visualize tab characters as `^I`. Implies -v.
//   -e      Visualize trailing newline as `$`.  Implies -v.
//   "-"     Interpreted as standard input.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::fs::File;
use std::io::{self, BufWriter, Read, Write};

/// Internal I/O buffer size for buffered mode.
const BUFSZ: usize = 256 * 1024;

/// Entry point for the `cat` builtin.
fn cat_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "uvte") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("cat: {e}");
            return 1;
        }
    };

    let flag_u = opts.count('u') > 0;
    let flag_v = opts.count('v') > 0;
    let flag_t = opts.count('t') > 0;
    let flag_e = opts.count('e') > 0;

    let fast_path = !flag_u && !flag_v && !flag_t && !flag_e;

    let mut exit_code: u8 = 0;
    let stdout = io::stdout();
    let mut out = stdout.lock();

    // No file arguments — read from stdin.
    if ctx.optargs.is_empty() {
        let mut stdin = io::stdin().lock();
        let res = if fast_path {
            io::copy(&mut stdin, &mut out).map(|_| ())
        } else {
            process(&mut stdin, flag_u, flag_v, flag_t, flag_e, &mut out)
        };
        if let Err(e) = res {
            eprintln!("cat: stdin: {e}");
            exit_code = 1;
        }
        return exit_code;
    }

    for name in &ctx.optargs {
        if name == "-" {
            let mut stdin = io::stdin().lock();
            let res = if fast_path {
                io::copy(&mut stdin, &mut out).map(|_| ())
            } else {
                process(&mut stdin, flag_u, flag_v, flag_t, flag_e, &mut out)
            };
            if let Err(e) = res {
                eprintln!("cat: stdin: {e}");
                exit_code = 1;
            }
        } else {
            match File::open(name) {
                Ok(mut file) => {
                    let res = if fast_path {
                        io::copy(&mut file, &mut out).map(|_| ())
                    } else {
                        process(&mut file, flag_u, flag_v, flag_t, flag_e, &mut out)
                    };
                    if let Err(e) = res {
                        eprintln!("cat: {name}: {e}");
                        exit_code = 1;
                    }
                }
                Err(e) => {
                    eprintln!("cat: {name}: {e}");
                    exit_code = 1;
                }
            }
        }
    }

    // Сброс stdout после всех операций (на случай, если process не до конца опустошил буфер)
    if let Err(e) = out.flush() {
        eprintln!("cat: stdout: {e}");
        exit_code = 1;
    }

    exit_code
}

/// Read from a reader and write its contents to `out` according to flags.
///
/// В быстром пути (u=v=t=e=false) не вызывается — вместо него работает io::copy.
/// При -u читает побайтово, при -v/-t/-e применяет визуализацию с буферизацией.
fn process<R: Read, W: Write>(
    reader: &mut R,
    u: bool,
    v: bool,
    t: bool,
    e: bool,
    out: &mut W,
) -> io::Result<()> {
    // Unbuffered mode: byte by byte, никакой буферизации.
    if u {
        let mut buf = [0u8; 1];
        loop {
            let len = reader.read(&mut buf)?;
            if len == 0 {
                break;
            }
            if v || t || e {
                write_visualized(buf[0], v, t, e, out)?;
            } else {
                out.write_all(&buf)?;
            }
        }
        return Ok(());
    }

    // Buffered visualization mode: BufWriter для снижения накладных расходов.
    // write_visualized пишет сразу в буферизированный поток, без сбора в Vec.
    let mut writer = BufWriter::with_capacity(BUFSZ, out);
    let mut buf = vec![0u8; BUFSZ];

    loop {
        let len = reader.read(&mut buf)?;
        if len == 0 {
            break;
        }
        for &b in &buf[..len] {
            write_visualized(b, v, t, e, &mut writer)?;
        }
    }
    writer.flush()?;
    Ok(())
}

/// Render a single byte to `out`, applying -v, -t, and -e transformations.
fn write_visualized<W: Write>(mut b: u8, v: bool, t: bool, e: bool, out: &mut W) -> io::Result<()> {
    if b > 126 && v {
        if b > 127 {
            out.write_all(b"M-")?;
            b -= 128;
        }
        if b == 127 {
            out.write_all(b"^?")?;
            return Ok(());
        }
    }

    if b < 32 {
        if b == b'\n' {
            if e {
                out.write_all(b"$")?;
            }
            out.write_all(b"\n")?;
        } else if (b == b'\t' && t) || (b != b'\t' && v) {
            let ch = (b + b'@') as char;
            out.write_all(b"^")?;
            out.write_all(&[ch as u8])?;
        } else {
            out.write_all(&[b])?;
        }
    } else {
        out.write_all(&[b])?;
    }

    Ok(())
}

register_command!(
    CAT_CMD,
    "cat",
    "uvte",
    CommandFlags::BIN.bits(),
    cat_main
);