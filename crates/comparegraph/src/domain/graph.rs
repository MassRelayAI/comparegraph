use crate::domain::NodeId;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    File,
    Module,
    Class,
    Function,
    Method,
    Type,
    Route,
    Test,
}

impl NodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Module => "module",
            Self::Class => "class",
            Self::Function => "function",
            Self::Method => "method",
            Self::Type => "type",
            Self::Route => "route",
            Self::Test => "test",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "file" => Self::File,
            "module" => Self::Module,
            "class" => Self::Class,
            "function" => Self::Function,
            "method" => Self::Method,
            "type" => Self::Type,
            "route" => Self::Route,
            "test" => Self::Test,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Contains,
    Imports,
    Calls,
    Implements,
    Tests,
    RoutesTo,
}

impl EdgeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Contains => "contains",
            Self::Imports => "imports",
            Self::Calls => "calls",
            Self::Implements => "implements",
            Self::Tests => "tests",
            Self::RoutesTo => "routes_to",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "contains" => Self::Contains,
            "imports" => Self::Imports,
            "calls" => Self::Calls,
            "implements" => Self::Implements,
            "tests" => Self::Tests,
            "routes_to" => Self::RoutesTo,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Node {
    pub id: Option<NodeId>,
    pub kind: NodeKind,
    pub name: String,
    pub qualified_name: String,
    pub path: String,
    pub line_start: u32,
    pub line_end: u32,
    pub content_hash: String,
    pub language: String,
    pub provider_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Edge {
    pub kind: EdgeKind,
    pub source: NodeId,
    pub target: NodeId,
    pub confidence: f64,
}

/// sha256 hex over the lines joined with '\n' (no trailing newline). Stable across platforms: callers pass lines, not raw bytes.
pub fn content_hash(lines: &[&str]) -> String {
    let mut h = Sha256::new();
    for (i, l) in lines.iter().enumerate() {
        if i > 0 {
            h.update(b"\n");
        }
        h.update(l.as_bytes());
    }
    hex::encode(h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn kinds_round_trip() {
        for k in [
            NodeKind::File,
            NodeKind::Module,
            NodeKind::Class,
            NodeKind::Function,
            NodeKind::Method,
            NodeKind::Type,
            NodeKind::Route,
            NodeKind::Test,
        ] {
            assert_eq!(NodeKind::parse(k.as_str()), Some(k));
        }
        for k in [
            EdgeKind::Contains,
            EdgeKind::Imports,
            EdgeKind::Calls,
            EdgeKind::Implements,
            EdgeKind::Tests,
            EdgeKind::RoutesTo,
        ] {
            assert_eq!(EdgeKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(NodeKind::parse("nope"), None);
    }
    #[test]
    fn content_hash_is_stable_and_line_sensitive() {
        let a = content_hash(&["fn x() {}", "fn y() {}"]);
        let b = content_hash(&["fn x() {}", "fn y() {}"]);
        let c = content_hash(&["fn x() {}", "fn z() {}"]);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 64);
    }
}
