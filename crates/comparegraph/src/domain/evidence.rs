#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Observed,
    Derived,
    Inferred,
    Unknown,
}
impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::Derived => "derived",
            Self::Inferred => "inferred",
            Self::Unknown => "unknown",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "observed" => Self::Observed,
            "derived" => Self::Derived,
            "inferred" => Self::Inferred,
            "unknown" => Self::Unknown,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Location {
    pub path: String,
    pub line_start: u32,
    pub line_end: u32,
    pub content_hash: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn confidence_round_trip() {
        for c in [
            Confidence::Observed,
            Confidence::Derived,
            Confidence::Inferred,
            Confidence::Unknown,
        ] {
            assert_eq!(Confidence::parse(c.as_str()), Some(c));
        }
    }
}
