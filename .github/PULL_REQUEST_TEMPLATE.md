## What and why

## Checklist
- [ ] Tests written first (a failing test existed before the fix)
- [ ] Fixture touched? If yes: `comparegraph conform reference --fixtures fixtures` still green, and the change is explained
- [ ] Design-note row added or updated for any behaviour change
- [ ] `cargo test -p comparegraph` and `cargo clippy -p comparegraph --tests -- -D warnings` pass locally
