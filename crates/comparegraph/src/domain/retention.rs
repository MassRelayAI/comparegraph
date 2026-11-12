//! The keep-rules of `gc` (pure). v1: the parse cache, by provider version; a stuck run, by age.

/// The provider versions `gc --parse-cache` keeps: the running binary's, plus the `keep - 1` others that sort last
/// (newest by version text). `keep` is at least 1. Sorted.
pub fn cache_versions_to_keep(present: &[String], current: &str, keep: usize) -> Vec<String> {
    let mut others: Vec<&String> = present.iter().filter(|v| v.as_str() != current).collect();
    others.sort();
    others.reverse();
    let mut out: Vec<String> = vec![current.to_string()];
    out.extend(others.into_iter().take(keep.saturating_sub(1)).cloned());
    out.sort();
    out.dedup();
    out
}

/// A run left `running` is abandoned when it started more than `max_age_secs` before `now_secs`.
pub fn run_is_abandoned(started_secs: Option<i64>, now_secs: i64, max_age_secs: i64) -> bool {
    started_secs.is_some_and(|s| now_secs - s > max_age_secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keeps_the_current_version_and_the_newest_others() {
        let p: Vec<String> = ["0.1.0+a", "0.1.1+a", "0.1.3+b"].iter().map(|s| s.to_string()).collect();
        assert_eq!(cache_versions_to_keep(&p, "0.1.3+b", 1), vec!["0.1.3+b"]);
        assert_eq!(cache_versions_to_keep(&p, "0.1.3+b", 2), vec!["0.1.1+a", "0.1.3+b"]);
        assert_eq!(cache_versions_to_keep(&p, "0.2.0+c", 1), vec!["0.2.0+c"], "a version with no rows is still the one kept");
    }
    #[test]
    fn age_limit_is_strict() {
        assert!(!run_is_abandoned(Some(0), 3600, 3600));
        assert!(run_is_abandoned(Some(0), 3601, 3600));
        assert!(!run_is_abandoned(None, 10_000, 1));
    }
}
