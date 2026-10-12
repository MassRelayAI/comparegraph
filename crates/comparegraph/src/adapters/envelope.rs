//! The one response envelope every verb answers with (spec §9b).
use crate::application::AppError;
use crate::domain::Limitation;
use serde_json::Value;

#[derive(serde::Serialize)]
pub struct Envelope {
    pub ok: bool,
    pub verb: String,
    pub schema: u32,
    pub data: Value,
    pub evidence: Evidence,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<Page>,
    pub cost: Cost,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
}
#[derive(serde::Serialize)]
pub struct Evidence {
    pub confidence: &'static str,
    pub limitations: Vec<Limitation>,
}
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Page {
    pub next: Option<String>,
    pub total: usize,
}
#[derive(serde::Serialize)]
pub struct Cost {
    pub rows: usize,
    pub bytes: usize,
}
#[derive(serde::Serialize)]
pub struct ErrorBody {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl Envelope {
    pub fn ok(
        verb: &str,
        data: Value,
        confidence: &'static str,
        limitations: Vec<Limitation>,
        page: Option<Page>,
    ) -> Self {
        Envelope {
            ok: true,
            verb: verb.to_string(),
            schema: 1,
            data,
            evidence: Evidence {
                confidence,
                limitations,
            },
            page,
            cost: Cost { rows: 0, bytes: 0 },
            error: None,
        }
    }
    pub fn err(verb: &str, e: &AppError) -> Self {
        Envelope {
            ok: false,
            verb: verb.to_string(),
            schema: 1,
            data: Value::Null,
            evidence: Evidence {
                confidence: "unknown",
                limitations: vec![],
            },
            page: None,
            cost: Cost { rows: 0, bytes: 0 },
            error: Some(ErrorBody {
                code: e.code(),
                message: e.to_string(),
                hint: None,
            }),
        }
    }
    /// Serialise with `cost` filled: bytes = length of the data JSON, rows = array length
    /// of `data` or `data.items`, else 1 (0 for a null/error body).
    pub fn to_json(&self) -> String {
        let bytes = serde_json::to_string(&self.data)
            .map(|s| s.len())
            .unwrap_or(0);
        let rows = match (&self.data, self.data.get("items")) {
            (Value::Array(a), _) => a.len(),
            (_, Some(Value::Array(a))) => a.len(),
            (Value::Null, _) => 0,
            _ => 1,
        };
        let mut v = serde_json::to_value(self).expect("envelope serialises");
        v["cost"] = serde_json::json!({ "rows": rows, "bytes": bytes });
        v.to_string()
    }
}
