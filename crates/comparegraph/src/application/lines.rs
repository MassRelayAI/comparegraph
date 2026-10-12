//! One place that turns a 1-based line span of a file's text into its content hash (used by show, check and claim).
use crate::domain::content_hash;

/// Hash of lines `ls..=le` of `src`; `None` when the span is out of range (`1 ≤ ls ≤ le ≤ lines`).
pub fn span_hash(src: &str, ls: usize, le: usize) -> Option<String> {
    let lines: Vec<&str> = src.lines().collect();
    if ls == 0 || ls > le || le > lines.len() {
        return None;
    }
    Some(content_hash(&lines[ls - 1..le]))
}
