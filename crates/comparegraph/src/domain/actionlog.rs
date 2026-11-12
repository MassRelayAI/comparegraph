//! The action log (`action/1`): one JSON object per line, append-only, hash-chained. Pure: canonical form, the hash of
//! an entry, and the walk over the bytes of a log handed in. The file and the head kept in `meta` are the application's.
//!
//! `hash = sha256_hex(prev || canonical(entry without "hash"))`; the first entry's `prev` is the empty string. Canonical
//! form: object keys sorted (recursively), no insignificant whitespace, numbers as written.
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

pub const SCHEMA: &str = "action/1";

/// `v` with every object's keys sorted, rendered compact.
pub fn canonical(v: &Value) -> String {
    fn sort(v: &Value) -> Value {
        match v {
            Value::Object(m) => {
                let mut keys: Vec<&String> = m.keys().collect();
                keys.sort();
                let mut out = Map::new();
                for k in keys {
                    out.insert(k.clone(), sort(&m[k]));
                }
                Value::Object(out)
            }
            Value::Array(a) => Value::Array(a.iter().map(sort).collect()),
            other => other.clone(),
        }
    }
    sort(v).to_string()
}

pub fn sha256_hex(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

/// The hash of an entry: over `prev` and the canonical entry without its `hash` key.
pub fn entry_hash(prev: &str, entry: &Value) -> String {
    let mut e = entry.clone();
    if let Some(o) = e.as_object_mut() {
        o.remove("hash");
    }
    let mut h = Sha256::new();
    h.update(prev.as_bytes());
    h.update(canonical(&e).as_bytes());
    hex::encode(h.finalize())
}

/// Give `entry` its `schema`, `seq`, `prev` and `hash` (the next link after `prev_hash` at `seq - 1`).
pub fn seal(mut entry: Map<String, Value>, seq: u64, prev_hash: &str) -> Value {
    entry.insert("schema".into(), Value::String(SCHEMA.into()));
    entry.insert("seq".into(), Value::from(seq));
    entry.insert("prev".into(), Value::String(prev_hash.into()));
    entry.remove("hash");
    let mut v = Value::Object(entry);
    let h = entry_hash(prev_hash, &v);
    v.as_object_mut().unwrap().insert("hash".into(), Value::String(h));
    v
}

/// Where the chain broke: the 1-based line and why.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Break {
    pub line: usize,
    pub seq: Option<u64>,
    pub reason: String,
}

/// The result of walking a log: its entries (up to the first break) and the head (`seq`, `hash`) of the last good one.
#[derive(Debug, Clone, Default)]
pub struct Walk {
    pub entries: Vec<Value>,
    pub head: Option<(u64, String)>,
    pub broken: Option<Break>,
}
impl Walk {
    /// The head as stored in `meta` (`<seq>:<hash>`), or None for an empty log.
    pub fn head_text(&self) -> Option<String> {
        self.head.as_ref().map(|(s, h)| format!("{s}:{h}"))
    }
}

/// Walk the bytes of a log: every line parses, `schema` is `action/1`, `seq` runs 1, 2, 3 ... without a gap, each `prev`
/// is the previous `hash` and each `hash` recomputes. Stops at the first break.
pub fn walk(text: &str) -> Walk {
    let mut w = Walk::default();
    let mut prev = String::new();
    let total = text.split('\n').count();
    for (i, line) in text.split('\n').enumerate() {
        let n = i + 1;
        if line.is_empty() {
            // only the final newline may leave an empty piece
            if n == total {
                break;
            }
            w.broken = Some(Break { line: n, seq: None, reason: "empty line".into() });
            return w;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                w.broken = Some(Break { line: n, seq: None, reason: format!("not JSON: {e}") });
                return w;
            }
        };
        let seq = v.get("seq").and_then(Value::as_u64);
        let expect = w.entries.len() as u64 + 1;
        let fail = |reason: String| Some(Break { line: n, seq, reason });
        if v.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
            w.broken = fail(format!("schema is not {SCHEMA}"));
            return w;
        }
        if seq != Some(expect) {
            w.broken = fail(format!("seq {seq:?}, expected {expect}"));
            return w;
        }
        if v.get("prev").and_then(Value::as_str) != Some(prev.as_str()) {
            w.broken = fail("prev is not the hash of the line before".into());
            return w;
        }
        let stored = v.get("hash").and_then(Value::as_str).unwrap_or("").to_string();
        if entry_hash(&prev, &v) != stored {
            w.broken = fail("hash does not match the entry".into());
            return w;
        }
        prev = stored.clone();
        w.head = Some((expect, stored));
        w.entries.push(v);
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn log(n: u64) -> String {
        let mut prev = String::new();
        let mut out = String::new();
        for s in 1..=n {
            let e = seal(json!({"verb":"gc","at":"t","by":"me","n":s}).as_object().unwrap().clone(), s, &prev);
            prev = e["hash"].as_str().unwrap().to_string();
            out.push_str(&e.to_string());
            out.push('\n');
        }
        out
    }
    #[test]
    fn canonical_sorts_keys_at_every_depth() {
        assert_eq!(canonical(&json!({"b":1,"a":{"d":2,"c":[{"z":1,"y":2}]}})), r#"{"a":{"c":[{"y":2,"z":1}],"d":2},"b":1}"#);
    }
    #[test]
    fn a_sealed_chain_walks_clean() {
        let w = walk(&log(3));
        assert!(w.broken.is_none(), "{:?}", w.broken);
        assert_eq!(w.entries.len(), 3);
        assert_eq!(w.head.as_ref().unwrap().0, 3);
        assert!(walk("").broken.is_none());
        assert!(walk("").head.is_none());
    }
    #[test]
    fn one_flipped_byte_breaks_at_its_line() {
        let t = log(4);
        let lines: Vec<&str> = t.lines().collect();
        let bad = lines[2].replacen("\"by\":\"me\"", "\"by\":\"mx\"", 1);
        let tampered = format!("{}\n{}\n{}\n{}\n", lines[0], lines[1], bad, lines[3]);
        let w = walk(&tampered);
        assert_eq!(w.broken.as_ref().unwrap().line, 3, "{:?}", w.broken);
        assert_eq!(w.head.unwrap().0, 2);
    }
    #[test]
    fn a_gap_in_seq_and_a_reordered_line_break_the_chain() {
        let t = log(3);
        let l: Vec<&str> = t.lines().collect();
        assert_eq!(walk(&format!("{}\n{}\n", l[0], l[2])).broken.unwrap().line, 2);
        assert_eq!(walk(&format!("{}\n{}\n", l[1], l[0])).broken.unwrap().line, 1);
    }
}
