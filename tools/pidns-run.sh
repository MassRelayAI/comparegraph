#!/usr/bin/env bash
# pidns-run.sh — run a program inside its own PID namespace, so a kill(-1) or a stray process-group kill from a test can
# reach only the test's own processes, never the desktop, the terminals or other sessions of the same user.
#
# Used as the cargo runner for this workspace (.cargo/config.toml): every test binary and `cargo run` goes through it.
# Needs unprivileged user namespaces. On Ubuntu 24.04 they are off by default (kernel.apparmor_restrict_unprivileged_userns=1);
# docs/dev/pid-namespaces.md has the one-time AppArmor rule. Without it this script falls back to running the program as is
# and says so once per invocation on stderr, so nothing breaks — it just is not confined.
#
# Why (2026-10-13): a test shelled out to /usr/bin/kill with a negative 7-digit pid, which procps parses as -1, and killed
# every process of the user five times. The code fix and the hooks stop the spelling; this script confines whatever slips through.
set -u
if [ "${COMPAREGRAPH_NO_PIDNS:-}" = 1 ]; then exec "$@"; fi
if unshare -Urpf --mount-proc --kill-child true 2>/dev/null; then
  exec unshare -Urpf --mount-proc --kill-child "$@"
fi
echo "pidns-run: PID namespaces unavailable (AppArmor); running unconfined — see docs/dev/pid-namespaces.md" >&2
exec "$@"
