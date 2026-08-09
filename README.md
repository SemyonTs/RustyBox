# RustyBox

[![Crates.io Version](https://img.shields.io/crates/v/rustybox_utils)](https://crates.io/crates/rustybox_utils)
[![Crates.io Downloads](https://img.shields.io/crates/d/rustybox_utils)](https://crates.io/crates/rustybox_utils)
[![Crates.io License](https://img.shields.io/crates/l/rustybox_utils)](https://crates.io/crates/rustybox_utils)

Rust implementation of common \*nix command-line utilities, inspired by [Toybox](https://landley.net/toybox/).

RustyBox provides 58 commands:

basename, cat, chmod, cp, cut, date, df, dirname, du, echo, env, false, grep, head, id, kill, link, ln, ls, mkdir, mv, printf, pwd, readlink, rm, rmdir, sed, sleep, sort, tail, tee, test, touch, tr, true, uname, unlink, wc, xargs, sh, cd, exit, exec, export, alias, jobs, fg, bg, eval, set, unset.


## Usage

1. Build: `cargo build --release`
2. List available commands: `./target/release/rustybox`
3. Install: `sudo cp target/release/rustybox /usr/local/bin/`

Create symlinks to use commands without the `rustybox` prefix (e.g. `rustybox ls` or `rustybox grep`).

## Benchmarks (release v0.1.2)

RustyBox continues to improve with each release. Below are the latest measurements taken on an **Intel Core i7-8550U** (frequency locked, performance governor) using `hyperfine` (microsecond precision). RustyBox was built with `RUSTFLAGS="-C target-cpu=generic"`. GNU Coreutils and BusyBox from Arch Linux official repositories, Toybox from AUR.

**Results (lower is better):**

| Command | Dataset | RustyBox | BusyBox | Toybox | GNU |
|---------|---------|----------|---------|--------|-----|
| `cat` | 100k lines | **0.001970s** | 0.001293s | 0.003636s | 0.001910s |
| `grep` | 100k lines | **0.009658s** | 0.100848s | 0.154130s | 0.002109s |
| `wc -l` | 1M lines | **0.063230s** | 0.183817s | 0.277668s | 0.009760s |
| `sort` | 100k numbers | **0.037165s** | 0.379018s | 0.194906s | 0.048222s |
| `sed` | 100k lines | **0.017466s** | 0.160299s | 0.094141s | 0.037459s |
| `tr` | 1M lines | **0.001900s** | 0.000820s | — (failed) | — |
| `cut` | 100k lines | **0.015056s** | 0.094118s | 0.023782s | 0.008933s |
| `cat\|grep\|wc` | 100k lines | **0.011697s** | 0.117627s | 0.209731s | 0.012360s |

**Summary:**

- GNU `cat` is essentially tied with RustyBox (1.03× faster); BusyBox is 1.52× faster than RustyBox; RustyBox is 1.85× faster than Toybox.
- RustyBox `grep` is **10.4× faster** than BusyBox and **16.0× faster** than Toybox (GNU is 4.58× faster).
- RustyBox `wc -l` is **2.9× faster** than BusyBox and **4.4× faster** than Toybox (GNU is 6.48× faster).
- RustyBox `sort` is **1.3× faster** than GNU, **5.2× faster** than Toybox, and **10.2× faster** than BusyBox.
- RustyBox `sed` is **2.1× faster** than GNU, **5.4× faster** than Toybox, and **9.2× faster** than BusyBox.
- RustyBox `tr` is **2.3× faster** than BusyBox (Toybox failed, GNU not measured in this run).
- RustyBox `cut` is **6.3× faster** than BusyBox, **1.6× faster** than Toybox (GNU is 1.69× faster).
- RustyBox pipeline (`cat|grep|wc`) is **10.1× faster** than BusyBox and **17.9× faster** than Toybox, and **slightly faster** than GNU (1.06×).

**Key takeaways:**

- RustyBox now **outperforms GNU** in `sort`, `sed`, and the pipeline (`cat|grep|wc`).
- It is **consistently faster** than BusyBox and Toybox across almost all tested commands.
- The gap to GNU in `grep` and `wc -l` has significantly narrowed compared to v0.1.1.
- All of this is achieved while using **standard POSIX libc interfaces**, making RustyBox portable to non‑Linux systems without sacrificing performance.

`sh` and built-in shell commands are no longer compiled by default. `rbsh` is not suitable for serious use.

## License

RustyBox is licensed under MPL-2.0. See `LICENSE` and `NOTICE` before use.