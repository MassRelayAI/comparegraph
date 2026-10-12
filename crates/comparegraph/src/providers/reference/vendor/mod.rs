//! The tree-sitter parser layer, taken whole from graphmind-core (MIT). See docs/THIRD_PARTY_NOTICES.md.
#[allow(clippy::all, dead_code)]
pub mod extractor;
#[allow(clippy::all, dead_code)]
pub mod languages;
#[allow(clippy::all, dead_code)]
pub mod parser;
#[allow(clippy::all, dead_code)]
pub mod registry;
#[allow(clippy::all, dead_code)]
pub mod resolver;
pub use extractor::{CallSite, Symbol, SymbolKind};
pub use parser::{parse, ParseResult};
pub use resolver::ResolvedImport;
