//! Licence lanes for `repos --no-port`: the never-copy set is a constant here; a policy may only EXTEND it
//! (`licences.never_copy_extra`), never shorten it, so a configuration typo cannot turn AGPL into `take`.

/// SPDX id prefixes whose code is never copied: copyleft and source-available licences.
pub const NEVER_COPY_PREFIXES: &[&str] = &["AGPL", "GPL", "LGPL", "SSPL", "BUSL", "EUPL", "OSL"];
/// Fair-code and other source-available licences matched by exact id (case-insensitive).
pub const NEVER_COPY_EXACT: &[&str] = &["fair-code", "SUL-1.0", "Sustainable-Use-1.0", "Elastic-2.0", "Commons-Clause", "n8n-ee"];
/// The lanes whose code may be ported (with attribution).
pub const PORT_LANES: &[&str] = &["take", "adapt"];

fn matches(id: &str, pat: &str) -> bool {
    let (a, b) = (id.to_ascii_uppercase(), pat.to_ascii_uppercase());
    a == b || (a.starts_with(&b) && a[b.len()..].starts_with(['-', '+', '.', ' ']))
}

/// Why a repo must not be ported from, empty when it may: `lane study`, `licence AGPL-3.0`, `licence unread`. Sorted.
pub fn no_port_reasons(licence: Option<&str>, lane: &str, extra: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    if !PORT_LANES.contains(&lane) {
        out.push(format!("lane {lane}"));
    }
    match licence.map(str::trim).filter(|l| !l.is_empty()) {
        None => out.push("licence unread".into()),
        Some(l) => {
            let hit = NEVER_COPY_PREFIXES.iter().any(|p| matches(l, p))
                || NEVER_COPY_EXACT.iter().any(|p| l.eq_ignore_ascii_case(p))
                || extra.iter().any(|p| matches(l, p));
            if hit {
                out.push(format!("licence {l}"));
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn copyleft_unread_and_study_are_flagged_and_permissive_take_is_not() {
        assert_eq!(no_port_reasons(Some("AGPL-3.0"), "study", &[]), vec!["lane study", "licence AGPL-3.0"]);
        assert_eq!(no_port_reasons(Some("GPL-3.0"), "take", &[]), vec!["licence GPL-3.0"]);
        assert_eq!(no_port_reasons(None, "study", &[]), vec!["lane study", "licence unread"]);
        assert!(no_port_reasons(Some("MIT"), "take", &[]).is_empty());
        assert!(no_port_reasons(Some("Apache-2.0"), "take", &[]).is_empty());
        // a prefix matches at a boundary only: `GPLX` is not GPL
        assert!(no_port_reasons(Some("GPLX"), "take", &[]).is_empty());
        assert_eq!(no_port_reasons(Some("MPL-2.0"), "take", &["MPL".into()]), vec!["licence MPL-2.0"]);
    }
}
