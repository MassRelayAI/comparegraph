# Confining tests in a PID namespace

**Why.** A test that shells out to `kill` with a negative process-group id, as in `kill -KILL -<pgid>`, is parsed by
procps-ng as `kill -1` on hosts whose pids exceed one million: procps keeps only the first digit of a negative argument that
follows a signal option. `kill(-1, SIGKILL)` reaches every process of the user, the desktop session included. The code in
this crate signals through `libc::kill` on a pid it computed, `tests/no_kill_shellout.rs` forbids any kill shell-out in
`src/`, and the contributor hooks block the spelling in shell commands and in code being written. None of that can see inside
a compiled binary from a dependency or a future mistake. A PID namespace can: inside it, `kill(-1)` reaches only the namespace.

**How.** `.cargo/config.toml` sets `tools/pidns-run.sh` as the runner for the Linux targets, so every test binary and
`cargo run` executes under `unshare -Urpf --mount-proc --kill-child`. When unprivileged user namespaces are not allowed, the
runner prints one line on stderr and runs the program unconfined, so a fresh checkout on any host still builds and tests.
`COMPAREGRAPH_NO_PIDNS=1` skips the namespace on purpose (a debugger, strace on the host side).

**Enabling it where user namespaces are restricted (one-time, needs root).** Some distributions restrict unprivileged user
namespaces through AppArmor (`kernel.apparmor_restrict_unprivileged_userns = 1`). The narrow fix is a profile that allows
`/usr/bin/unshare` to create them, leaving the restriction in place for every other program:

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
confined by this runner; the hooks and the code rules are the guard there. A separate build user, or running test binaries in
a container, are the heavier alternatives.
