//! `relevance`: the factor counts the engine computes, the policy the user owns (`relevance-policy/1`), and the pure
//! score and class. The engine ships no weights: with no policy every repo is `unrated` (roadmap ruling 2026-10-13,
//! "a prioritization formula does NOT go in the tool"; recorded in docs/decisions.md). No I/O here.
use serde::{Deserialize, Serialize};

pub const POLICY_SCHEMA: &str = "relevance-policy/1";

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Weights {
    #[serde(rename = "use")]
    pub use_: f64,
    pub pull: f64,
    pub recency: f64,
    pub band: f64,
    pub ui: f64,
}
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Saturate {
    pub evidence_claims: f64,
    pub pull_rows: f64,
}
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecencyStep {
    pub max_days: i64,
    pub share: f64,
}
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Classes {
    pub keep_min: f64,
    pub remove_max: f64,
    pub grace_days: i64,
}
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Primary {
    pub repo: String,
    pub domain: String,
    pub why: String,
    pub by: String,
    pub date: String,
}
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Pin {
    pub repo: String,
    pub class: String,
    pub why: String,
    pub by: String,
    pub date: String,
    /// the last day (YYYY-MM-DD) the pin holds; after it the pin is listed as expired and ignored
    pub until: String,
}
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Only {
    pub column: String,
    #[serde(rename = "in")]
    pub values: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PullSpec {
    pub file: String,
    pub repo_column: String,
    /// count only the rows whose `column` holds one of `in` (e.g. open tracker states)
    #[serde(default)]
    pub only: Option<Only>,
}
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AliasSpec {
    pub file: String,
    pub from: String,
    pub to: String,
}
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CitedSpec {
    pub file: String,
    pub repo_column: String,
    pub date_column: String,
}
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UiSpec {
    pub file: String,
}
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Retention {
    pub parse_cache_versions: usize,
}
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Licences {
    /// SPDX ids or prefixes (`EUPL`, `CC-BY-NC-4.0`) added to the never-copy set; the set can only grow
    #[serde(default)]
    pub never_copy_extra: Vec<String>,
}

/// `relevance-policy/1`. Unknown keys are refused (`invalid_policy`), never ignored.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub schema: String,
    #[serde(default)]
    pub version: Option<u64>,
    /// free text for the reader of the file; never read by the engine
    #[serde(default)]
    pub note: Option<String>,
    pub weights: Weights,
    pub saturate: Saturate,
    pub recency: Vec<RecencyStep>,
    pub classes: Classes,
    #[serde(default)]
    pub primaries: Vec<Primary>,
    #[serde(default)]
    pub pins: Vec<Pin>,
    #[serde(default)]
    pub pull: Option<PullSpec>,
    #[serde(default)]
    pub aliases: Option<AliasSpec>,
    #[serde(default)]
    pub cited: Option<CitedSpec>,
    #[serde(default)]
    pub ui: Option<UiSpec>,
    #[serde(default)]
    pub retention: Option<Retention>,
    #[serde(default)]
    pub licences: Option<Licences>,
}

/// The classes a pin may name.
pub const PIN_CLASSES: &[&str] = &["KEEP", "KEEP-PRIMARY", "KEEP-NEW", "ARCHIVE", "REMOVE-CANDIDATE"];
/// Class order for sorting (first is shown first).
pub const CLASS_ORDER: &[&str] = &["ROOT", "KEEP-PRIMARY", "KEEP", "KEEP-NEW", "ARCHIVE", "REMOVE-CANDIDATE", "unrated"];

impl Policy {
    /// Parse and validate. The message names what is wrong.
    pub fn parse(text: &str) -> Result<Policy, String> {
        let p: Policy = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if p.schema != POLICY_SCHEMA {
            return Err(format!("schema is {:?}, expected {POLICY_SCHEMA:?}", p.schema));
        }
        let w = &p.weights;
        if [w.use_, w.pull, w.recency, w.band, w.ui].iter().any(|x| !x.is_finite() || *x < 0.0) {
            return Err("weights must be finite and >= 0".into());
        }
        if !(p.saturate.evidence_claims > 0.0 && p.saturate.pull_rows > 0.0) {
            return Err("saturate values must be > 0".into());
        }
        if p.recency.iter().any(|s| s.max_days < 0 || !(0.0..=1.0).contains(&s.share)) {
            return Err("recency steps need max_days >= 0 and share in 0..=1".into());
        }
        if p.classes.remove_max > p.classes.keep_min || p.classes.grace_days < 0 {
            return Err("classes need remove_max <= keep_min and grace_days >= 0".into());
        }
        for pin in &p.pins {
            if !PIN_CLASSES.contains(&pin.class.as_str()) {
                return Err(format!("pin {}: class {} is not one of {}", pin.repo, pin.class, PIN_CLASSES.join(", ")));
            }
            if parse_date_days(&pin.until).is_none() {
                return Err(format!("pin {}: until {:?} is not a YYYY-MM-DD date", pin.repo, pin.until));
            }
        }
        if p.retention.as_ref().is_some_and(|r| r.parse_cache_versions == 0) {
            return Err("retention.parse_cache_versions must be >= 1".into());
        }
        Ok(p)
    }
    pub fn primary(&self, repo: &str) -> Option<&Primary> {
        self.primaries.iter().find(|x| x.repo == repo)
    }
    pub fn pin(&self, repo: &str) -> Option<&Pin> {
        self.pins.iter().find(|x| x.repo == repo)
    }
}

/// How a repo is listed in the UI registry the policy names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UiRef {
    No,
    Listed,
    Proven,
}

/// The engine's part: counts, never weights.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Factors {
    /// live claims with verdict has or partial
    pub evidence_claims: u64,
    /// live claims with verdict none or unknown (reported, not scored)
    pub other_claims: u64,
    pub pull_rows: u64,
    /// days from the last citation (claim or the policy's `cited` file) to as-of; None when never cited
    pub days_since_cited: Option<i64>,
    /// the best band over the families where the repo has a live claim: related | adjacent | unrelated
    pub best_band: Option<String>,
    pub ui: UiRef,
    /// days from `added_at` to as-of
    pub days_since_added: Option<i64>,
}

/// Every term of the score, as `--explain` prints it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Terms {
    #[serde(rename = "use")]
    pub use_: f64,
    pub pull: f64,
    pub recency: f64,
    pub band: f64,
    pub ui: f64,
    pub total: f64,
}

/// Score 0..=sum(weights): `U + P + R + B + X` (CG-CLEANUP-PLAN section 1.3).
pub fn score(f: &Factors, p: &Policy) -> Terms {
    let w = &p.weights;
    let use_ = w.use_ * (f.evidence_claims as f64 / p.saturate.evidence_claims).min(1.0);
    let pull = w.pull * (f.pull_rows as f64 / p.saturate.pull_rows).min(1.0);
    let recency = match f.days_since_cited {
        Some(d) => p.recency.iter().find(|s| s.max_days >= d).map_or(0.0, |s| w.recency * s.share),
        None => 0.0,
    };
    let band = match f.best_band.as_deref() {
        Some("related") => w.band,
        Some("adjacent") => w.band / 2.0,
        _ => 0.0,
    };
    let ui = match f.ui {
        UiRef::Proven => w.ui,
        UiRef::Listed => w.ui / 2.0,
        UiRef::No => 0.0,
    };
    let r = |x: f64| (x * 1e6).round() / 1e6;
    Terms { use_: r(use_), pull: r(pull), recency: r(recency), band: r(band), ui: r(ui), total: r(use_ + pull + recency + band + ui) }
}

/// The class and why, first match wins: ROOT, a live pin, KEEP-PRIMARY, KEEP, KEEP-NEW, REMOVE-CANDIDATE, ARCHIVE.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Rating {
    pub class: String,
    /// the rule that gave the class
    pub rule: String,
    pub pinned: bool,
    /// a pin that has passed its `until` at as-of (listed, ignored)
    pub pin_expired: bool,
}

pub fn classify(repo: &str, is_root: bool, f: &Factors, t: &Terms, p: &Policy, as_of_days: i64) -> Rating {
    let mk = |class: &str, rule: &str| Rating { class: class.into(), rule: rule.into(), pinned: false, pin_expired: false };
    if is_root {
        return mk("ROOT", "repo.role = root");
    }
    let mut expired = false;
    if let Some(pin) = p.pin(repo) {
        if parse_date_days(&pin.until).is_some_and(|u| as_of_days <= u) {
            return Rating { class: pin.class.clone(), rule: format!("pinned by {} until {}: {}", pin.by, pin.until, pin.why), pinned: true, pin_expired: false };
        }
        expired = true;
    }
    let mut r = if let Some(pr) = p.primary(repo) {
        mk("KEEP-PRIMARY", &format!("primary for {} ({})", pr.domain, pr.why))
    } else if t.total >= p.classes.keep_min {
        mk("KEEP", &format!("score {} >= keep_min {}", t.total, p.classes.keep_min))
    } else if f.days_since_added.is_some_and(|d| d <= p.classes.grace_days) {
        mk("KEEP-NEW", &format!("added {} day(s) before as-of, within grace_days {}", f.days_since_added.unwrap(), p.classes.grace_days))
    } else if t.total < p.classes.remove_max && f.evidence_claims == 0 && f.pull_rows == 0 {
        mk("REMOVE-CANDIDATE", &format!("score {} < remove_max {}, no evidence claim, no pull row", t.total, p.classes.remove_max))
    } else {
        mk("ARCHIVE", &format!("score {} < keep_min {}", t.total, p.classes.keep_min))
    };
    r.pin_expired = expired;
    r
}

/// Days since 1970-01-01 of a `YYYY-MM-DD` date (or the date part of an RFC 3339 timestamp).
pub fn parse_date_days(s: &str) -> Option<i64> {
    let d = s.get(..10)?;
    let mut it = d.split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: i64 = it.next()?.parse().ok()?;
    let dd: i64 = it.next()?.parse().ok()?;
    if it.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&dd) || d.as_bytes()[4] != b'-' || d.as_bytes()[7] != b'-' {
        return None;
    }
    // days from civil (Howard Hinnant)
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + dd - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}
/// Seconds since the epoch of an RFC 3339 UTC timestamp `YYYY-MM-DDTHH:MM:SSZ` (the form `now_iso` writes).
pub fn parse_ts_secs(s: &str) -> Option<i64> {
    let days = parse_date_days(s)?;
    let t = s.get(11..19)?;
    let mut it = t.split(':');
    let (h, m, sec): (i64, i64, i64) = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
    Some(days * 86_400 + h * 3600 + m * 60 + sec)
}
/// The name form two sources are matched on: lower case, letters and digits only.
pub fn norm_name(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn policy() -> Policy {
        Policy::parse(r#"{"schema":"relevance-policy/1","weights":{"use":40,"pull":25,"recency":15,"band":10,"ui":10},
            "saturate":{"evidence_claims":25,"pull_rows":3},"recency":[{"max_days":30,"share":1.0},{"max_days":60,"share":0.53}],
            "classes":{"keep_min":45,"remove_max":15,"grace_days":14},
            "primaries":[{"repo":"prim","domain":"agent loop","why":"D405","by":"founder","date":"2026-11-09"}],
            "pins":[{"repo":"pinned","class":"KEEP","why":"own model","by":"founder","date":"2026-11-12","until":"2026-12-01"}]}"#).unwrap()
    }
    fn f(claims: u64, pull: u64, cited: Option<i64>, added: Option<i64>) -> Factors {
        Factors { evidence_claims: claims, other_claims: 0, pull_rows: pull, days_since_cited: cited, best_band: None, ui: UiRef::No, days_since_added: added }
    }
    fn class(repo: &str, x: &Factors, p: &Policy, as_of: &str) -> String {
        let t = score(x, p);
        classify(repo, false, x, &t, p, parse_date_days(as_of).unwrap()).class
    }
    #[test]
    fn unknown_keys_and_a_wrong_schema_are_refused() {
        let base = serde_json::to_value(policy()).unwrap();
        let mut v = base.clone();
        v["weightz"] = serde_json::json!(1);
        assert!(Policy::parse(&v.to_string()).unwrap_err().contains("weightz"));
        let mut v = base.clone();
        v["licences"] = serde_json::json!({"never_copy": ["MIT"]});
        assert!(Policy::parse(&v.to_string()).is_err(), "the never-copy set can only be extended");
        let mut v = base;
        v["schema"] = serde_json::json!("relevance-policy/2");
        assert!(Policy::parse(&v.to_string()).unwrap_err().contains("expected"));
    }
    #[test]
    fn keep_min_boundary_lands_on_each_side() {
        let mut p = policy();
        p.weights = Weights { use_: 45.0, pull: 0.0, recency: 0.0, band: 0.0, ui: 0.0 };
        assert_eq!(class("x", &f(25, 0, None, Some(100)), &p, "2026-11-12"), "KEEP");
        p.weights.use_ = 44.9;
        assert_eq!(class("x", &f(25, 0, None, Some(100)), &p, "2026-11-12"), "ARCHIVE");
    }
    #[test]
    fn grace_days_14_is_new_and_15_is_judged() {
        let p = policy();
        assert_eq!(class("x", &f(0, 0, None, Some(14)), &p, "2026-11-12"), "KEEP-NEW");
        assert_eq!(class("x", &f(0, 0, None, Some(15)), &p, "2026-11-12"), "REMOVE-CANDIDATE");
    }
    #[test]
    fn a_pin_holds_on_its_last_day_and_expires_the_day_after() {
        let p = policy();
        assert_eq!(class("pinned", &f(0, 0, None, Some(100)), &p, "2026-12-01"), "KEEP");
        let x = f(0, 0, None, Some(100));
        let t = score(&x, &p);
        let r = classify("pinned", false, &x, &t, &p, parse_date_days("2026-12-02").unwrap());
        assert_eq!(r.class, "REMOVE-CANDIDATE");
        assert!(r.pin_expired);
    }
    #[test]
    fn remove_candidate_needs_no_claim_and_no_pull_row() {
        let p = policy();
        assert_eq!(class("x", &f(0, 0, None, Some(100)), &p, "2026-11-12"), "REMOVE-CANDIDATE");
        assert_eq!(class("x", &f(1, 0, None, Some(100)), &p, "2026-11-12"), "ARCHIVE");
        assert_eq!(class("x", &f(0, 1, None, Some(100)), &p, "2026-11-12"), "ARCHIVE");
        assert_eq!(class("prim", &f(0, 0, None, Some(100)), &p, "2026-11-12"), "KEEP-PRIMARY");
        let x = f(0, 0, None, None);
        let t = score(&x, &p);
        assert_eq!(classify("anything", true, &x, &t, &p, 0).class, "ROOT");
    }
    #[test]
    fn score_terms_follow_the_plan() {
        let p = policy();
        let mut x = f(50, 1, Some(45), Some(100));
        x.best_band = Some("adjacent".into());
        x.ui = UiRef::Listed;
        let t = score(&x, &p);
        assert_eq!((t.use_, t.recency, t.band, t.ui), (40.0, 7.95, 5.0, 5.0));
        assert!((t.pull - 25.0 / 3.0).abs() < 1e-5);
        assert_eq!(score(&f(0, 0, Some(61), None), &p).recency, 0.0);
    }
    #[test]
    fn dates_parse_to_days() {
        assert_eq!(parse_date_days("1970-01-01"), Some(0));
        assert_eq!(parse_date_days("2026-11-12T10:00:00Z").unwrap() - parse_date_days("2026-10-29").unwrap(), 14);
        assert_eq!(parse_date_days("2026-13-01"), None);
        assert_eq!(parse_ts_secs("1970-01-02T00:00:01Z"), Some(86_401));
    }
}
