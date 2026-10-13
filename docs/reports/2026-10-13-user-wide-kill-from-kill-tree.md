---
title: Five desktop logouts on the build host were kill(-1, SIGKILL) from comparegraph's external-provider kill_tree
date: 2026-10-13
valid_until: 2027-04-13
lane: plan-6-lane-c1 (worktree comparegraph-wt-6c)
verdict: NOT the GPU, NOT a Claude account ban, NOT memory. `kill_tree` in crates/comparegraph/src/providers/external.rs ran `/usr/bin/kill -KILL -<pid>`; procps-ng 4.0.4 keeps only the first digit of a negative pid that follows a `-SIG` option, so with pids above 1,000,000 (the build host is at ~1.6M, pid_max 4,194,304) the call became kill(-1, SIGKILL) and killed every process of the user — the GNOME session, every terminal and Claude session, and two services that run as the same user. Fixed by `kill -s KILL -- -<pid>`.
status: decided — fix applied uncommitted in comparegraph-wt-6c; lane owner commits it with the C1 work
---

## Symptom
Desktop session on :1 ended at 10:26, 10:35, 10:38, 10:41 and 10:48 (`last -x`). The user saw it as "whole account logged out"
and "terminal killed". Chrome/Firefox died with it. Claude Code auth was never touched (token refreshed normally 10:27, valid to 18:27).

## Evidence chain
1. `journalctl --user` has zero orderly-stop lines before any death; Xorg.1.log.old ends mid-startup with no "Server terminated" → SIGKILL, not a logout.
2. two system services that run as the same user outside the desktop session show `NRestarts=5` with restarts at 10:35, 10:38, 10:40, 10:48 → the kill hit every process of the uid, not a cgroup. Rules out systemd-oomd (only kills under the user's systemd slice; `oom_kill 0` in /proc/vmstat and memory.events), logind, GDM, the NVIDIA 470 driver.
3. Every death is within ~20 s of `cargo test -p comparegraph` in `comparegraph-wt-6c` (agent C1 at 10:38:24 and 10:40:43; orchestrator at 10:47:47). Both captured logs (`/tmp/claude-1001/t.out`, `scratchpad/c1-test.out`) stop at the same line: inside `tests/external.rs`, after its first test, where `a_hanging_provider_times_out` and `timeout_kills_grandchildren_too` reach `kill_tree`.
4. Parse proof — host, harmless signal 0 under strace: `/usr/bin/kill -0 -1597919` → `kill(-1, 0)`; `/usr/bin/kill -0 -- -1597919` → `kill(-1597919, 0)`. Container: `kill -KILL -2554` → `kill(-2, SIGKILL)`; `kill -9 -189` killed the whole container.
5. Red/green in a privileged ubuntu:24.04 container with `ns_last_pid=1500000` and a sentinel `sleep`: old binary `external-e1850bfa59febd8d` → sentinel and the test runner killed (exit 137). Patched binary `external-32a1abee81bb0ca8` → both tests pass, strace shows `kill(-1500021, SIGKILL)` and `kill(-1500038, SIGKILL)`, sentinel alive.

## Fix
`crates/comparegraph/src/providers/external.rs` `kill_tree`: `Command::new("kill").args(["-s", "KILL", "--", &format!("-{}", child.id())])`. A doc comment cites this report. Applied in the wt-6c working tree, uncommitted (the lane owner commits with C1). Any other `kill -SIG -<pgid>` spelling in the repos must get the `--` too.

## Detector
A lane test that spawns a sentinel outside the child's group and asserts it survives the timeout path would only go red on a host with 7-digit pids; the container recipe above (ns_last_pid=1500000, sentinel) is the repeatable red. Do not run the unfixed external tests on the build host.

