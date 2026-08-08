errata is a list of known vulnerabilities or
 instances where RustyBox's behavior deviates
 from what is obvious, implied, or required,
 along with corresponding PoCs.

1. Incomplete parsing in sh.
```
a@host   ~/..../rustybox $ RUST_BACKTRACE=1 /home/a/Source/rustybox/target/debug/rustybox cal
sh: RUST_BACKTRACE=1: command not found
```
```
root@freebsd   ~/..../rustybox $ RUST_BACKTRACE=1 /home/a/Source/rustybox/target/debug/rustybox cal
sh: RUST_BACKTRACE=1: command not found
```
```
a@host   ~/..../release $ echo "$(date +%F)"
(date +%F)
```
```
root@freebsd   ~/..../release $ echo "$(date +%F)"
(date +%F)
```

2. A number of id_ tests expect an ID > 0, which may yield a positive result when run as root.

3. Do not support here doc now