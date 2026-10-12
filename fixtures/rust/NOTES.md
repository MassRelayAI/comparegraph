# Rust fixture notes

`Cargo.toml::package` is a node because the vendored TOML extractor emits the package table; it is real. The trait method `Area::area` and the impl method `Square::area` share one qualified name (`src/shape.rs::area`); `total -> area` therefore resolves to it.

Limitation (not in our layer's small-fix reach): `tests/util.rs` has `use demo::util::double;`, a path through the package's own crate name. `rust_ctx::resolve_path` handles `crate`/`self`/`super`/module-relative paths only, so the true edge `imports tests/util.rs src/util.rs` is missed (it needs Cargo.toml crate-name resolution). It stays in expected.json; edge recall is 16/17. The `calls` and `tests` edges from the integration test are found by name.
