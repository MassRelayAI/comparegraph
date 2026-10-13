# Confining tests in a PID namespace

**Why.** On 2026-10-13 a test in this crate shelled out `kill -KILL -<pid>`. procps keeps only the first digit of a negative
pid that follows a signal option, so on a host whose pids have seven digits the call became `kill(-1, SIGKILL)`: every process
of the user, desktop included, five times (`docs/reports/2026-10-13-user-wide-kill-from-kill-tree.md`). The code now signals
through `libc::kill` on a pid it computed, `tests/no_kill_shellout.rs` forbids any kill shell-out in `src/`, and a Claude Code
hook blocks the spellings in shell commands and in code being written. None of that can see inside a compiled binary from
another repo or a future mistake. A PID namespace can: inside it, `kill(-1)` reaches only the namespace.

**How.** `.cargo/config.toml` sets `tools/pidns-run.sh` as the runner for the Linux target, so every test binary and
`cargo run` executes under `unshare -Urpf --mount-proc --kill-child`. When unprivileged user namespaces are not allowed, the
runner prints one line on stderr and runs the program unconfined, so a fresh checkout on any host still builds and tests.
`COMPAREGRAPH_NO_PIDNS=1` skips the namespace on purpose (a debugger, strace on the host side).

**Ubuntu 24.04: enabling it (one-time, needs sudo).** Unprivileged user namespaces are restricted by AppArmor
(`kernel.apparmor_restrict_unprivileged_userns = 1`). The narrow fix is a profile that allows `/usr/bin/unshare` to create
user namespaces, leaving the restriction in place for every other program:

```sh
sudo tee /etc/apparmor.d/usr.bin.unshare >/dev/null <<'EOF'
abi <abi/4.0>,
include <tunables/global>
profile unshare /usr/bin/unshare flags=(unconfined) {
  userns,
}
EOF
sudo apparmor_parser -r /etc/apparmor.d/usr.bin.unshare
```

Check: `unshare -Urpf --mount-proc sh -c 'echo pid $$; ps -e | wc -l'` prints `pid 1` and a handful of processes. The wide
alternative, `sudo sysctl kernel.apparmor_restrict_unprivileged_userns=0`, re-enables user namespaces for every program and
is not recommended on a shared host.

**What it does not cover.** A program run outside cargo (a shell script, a Python tool, a binary from another repo) is not
confined by this runner; the hook and the code rules are the guard there. A separate build user for agent work, or running
test binaries in a container, are the heavier alternatives listed in the report.
