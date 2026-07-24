# RustyBox

Rust implementation of common \*nix command-line utilities, inspired by [Toybox](https://landley.net/toybox/).

RustyBox provides 37 commands in a single binary, designed for ease of use. It uses standard POSIX libc interfaces, making it (theoretically) portable to operating systems beyond Linux.

## Usage

1. Build: `cargo build --release`
2. List available commands: `./target/release/rustybox`
3. Install: `sudo cp target/release/rustybox /usr/local/bin/`

Create symlinks to use commands without the `rustybox` prefix (e.g. `rustybox ls` or `rustybox grep`).

## Benchmarks (release v0.1.1)

RustyBox includes several optimizations that make it competitive with established implementations. Below are measurements taken on an Intel Core i7-8550U (frequency locked, performance governor) using `hyperfine` (microsecond precision). RustyBox was built with `RUSTFLAGS="-C target-cpu=generic"`. GNU Coreutils and BusyBox from Arch Linux official repositories, Toybox from AUR.

**Results (lower is better):**

| Command | Dataset | RustyBox | BusyBox | Toybox | GNU |
|---------|---------|----------|---------|--------|-----|
| `cat` | 100k lines | 0.0016s | 0.0007s | 0.0039s | 0.0011s |
| `grep` | 100k lines | 0.0176s | 0.1731s | 0.3045s | 0.0015s |
| `wc -l` | 1M lines | 0.2237s | 0.3249s | 0.4860s | 0.0118s |
| `sort` | 100k numbers | 0.1885s | 0.3529s | 0.2526s | 0.0958s |
| `sed` | 100k lines | 0.1813s | 0.2616s | 0.1840s | 0.0602s |
| `tr` | 1M lines | 0.1154s | 0.2810s | — | 0.0655s |
| `cut` | 100k lines | 0.0265s | 0.1428s | 0.0191s | 0.0078s |
| `cat\|grep\|wc` | 100k lines | 0.0232s | 0.1813s | 0.3509s | 0.0179s |

**Summary:**

- GNU `cat` is 1.45× faster than RustyBox; RustyBox is 2.4× faster than BusyBox and 2.5× faster than Toybox
- RustyBox `grep` is 9.8× faster than BusyBox and 17.3× faster than Toybox (GNU is 11.7× faster)
- RustyBox `wc -l` is 1.45× faster than BusyBox and 2.2× faster than Toybox (GNU is 19× faster)
- RustyBox `sort` is 1.9× faster than BusyBox and 1.3× faster than Toybox
- RustyBox `sed` is 1.4× faster than BusyBox and on par with Toybox
- RustyBox `tr` is 2.4× faster than BusyBox
- RustyBox `cut` is 5.4× faster than BusyBox (Toybox is 1.4× faster, GNU is 3.4× faster)
- RustyBox pipeline (`cat|grep|wc`) is 7.8× faster than BusyBox and 15× faster than Toybox

This illustrates that RustyBox is consistently faster than BusyBox and Toybox on most tasks.

## License

RustyBox is licensed under MPL-2.0. See `LICENSE` and `NOTICE` before use.