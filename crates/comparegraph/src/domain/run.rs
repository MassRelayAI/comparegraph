use crate::domain::{RepoId, RunId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Done,
    Failed,
}
impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "running" => Self::Running,
            "done" => Self::Done,
            "failed" => Self::Failed,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Limitation {
    pub code: String,
    pub detail: String,
}
impl Limitation {
    pub fn new(code: &str, detail: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Run {
    pub id: Option<RunId>,
    pub repo: RepoId,
    pub provider: String,
    pub provider_version: String,
    pub started_at: String,
    pub seconds: f64,
    pub status: RunStatus,
    pub limitations: Vec<Limitation>,
    pub store_hash: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_round_trip() {
        for s in [RunStatus::Running, RunStatus::Done, RunStatus::Failed] {
            assert_eq!(RunStatus::parse(s.as_str()), Some(s));
        }
    }
    #[test]
    fn limitations_serialize_as_json_array() {
        let v = serde_json::to_string(&vec![Limitation::new("skipped_file", "a.bin: not utf-8")])
            .unwrap();
        assert_eq!(
            v,
            r#"[{"code":"skipped_file","detail":"a.bin: not utf-8"}]"#
        );
    }
}
