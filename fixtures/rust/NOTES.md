# Rust fixture notes

`Cargo.toml::package` is a node because the vendored TOML extractor emits the package table; it is real. The trait method `Area::area` and the impl method `Square::area` share one qualified name (`src/shape.rs::area`); `total -> area` therefore resolves to it.

`tests/util.rs` has `use demo::util::double;`, a path through the package's own crate name. `rust_ctx::CrateMap` reads `[package] name` from `Cargo.toml` and resolves the first segment like `crate`, so `imports tests/util.rs src/util.rs` is found; edge recall is 17/17.
