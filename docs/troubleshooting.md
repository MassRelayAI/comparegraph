# Troubleshooting

Symptoms and their fixes.

| Symptom | Cause and fix |
|---|---|
| `no workspace` or `not_found` on a read verb | read verbs never create `.comparegraph`. Run `comparegraph add <clone>` in the workspace first, or pass `--root`. |
| `setup` says "running from a build directory" | a `target/` binary refuses to register itself. Pass `--bin <installed path>` or set `COMPAREGRAPH_BIN`. |
| `install.sh` says "no prebuilt binary for <platform>" | five platforms are prebuilt. Elsewhere build from source: `cargo install --locked --git https://github.com/massrelay-ai/comparegraph comparegraph`. |
| `install.sh` says "checksum mismatch" | the download does not match the release's `SHA256SUMS`; nothing was installed. Retry once; if it repeats, report it (`SECURITY.md`). |
| A claim shows `unknown*` or `stale` after you edited the clone | a location goes stale only when `comparegraph check` runs. Run `check`, rebuild, and write the claim again. |
| `conform` returns `below_floor` | the provider scored under 0.95 on nodes or 0.85 on edges. Fix the provider; the floors do not move. |
| `build --provider crg` says `provider_unavailable` | code-review-graph is not installed, or its conformance failed for a language in the clone. `comparegraph doctor` names which. |
| The session-start hook prints nothing | it is silent outside a workspace with a store. Run `comparegraph doctor`. |
| The agent does not see the server | run `comparegraph doctor`. `missing`: `setup --apply`. `stale` or `conflict`: add `--force`. Restart the client. With the plugin, `comparegraph` must be on your `PATH`. |
| A large repository is cut short | the run records `truncated_files` or `skipped_oversize`. Raise `--max-files` or `--max-file-bytes` and rebuild. |
| `search` for a word returns 0 hits | search matches whole identifier tokens, so `reader` does not find `ReaderConfig`. Try another stem. |
| `claim ... none` is refused | the engine's search found a hit (it lists the candidates), or the searched text named no token of the feature. |
| Tests note that the PID namespace is unavailable | contributors only: user namespaces are restricted on this host; tests still run unconfined. See [`docs/dev/pid-namespaces.md`](dev/pid-namespaces.md). |


[Back to README](../README.md)
