//! `ui net-diff`: the requests a page makes, ours against a reference: added, removed, changed.
//!
//! Source of the records, best first: `net.behaviour` (`net-behaviour/1`, written by the probe with `--net-behaviour`: every request a fingerprinted
//! transaction) when BOTH captures carry it; else the `net` block (failed calls and totals), which every capture since CMP-UI-1e has.
//! `net.shapes` (probe `--shapes`, value-free response shapes) add `shape` items for a request both sides made whose response shape differs.
//! `net.completeness` (the probe's REA completeness ledger) is always read: a capture that dropped events can show a difference but never "unchanged".
//!
//! Ported idea, not code: REA (MIT, github.com/morluto/rea, commit a016743) compares two captures as multisets of transaction fingerprints, pairs the
//! leftovers by request key into "changed", and says `unknown` instead of `unchanged` when ancestry or completeness cannot support the claim
//! (`src/domain/browserScenarioNetworkComparison.ts`, `browserScenarioNormalization.ts`). The probe (tools/ui/netdiff.mjs) already adapts that to JS; this
//! file is an independent Rust implementation of the comparison over our evidence types; see docs/THIRD_PARTY_NOTICES.md. Fingerprints are trusted as
//! stored (their canonical digest is the probe's); CompareGraph never recomputes them.
#![allow(clippy::result_large_err)] // serde_json::Value is larger with arbitrary_precision
use super::ui::{drift_error, drift_limitation, evidence_drift, need_token, open, seat_rank, Loaded, Opts, EV};
use super::{AppError, Result};
use crate::domain::ui::{Net, NetEvent, NetShape, NetTx};
use crate::domain::Limitation;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Items listed per capture at most; the summary still counts them all.
pub const MAX_ITEMS: usize = 200;

/// `METHOD path [type(, third-party)]`: what a request is, without its outcome.
fn key_of(t: &NetTx) -> String {
    match t.events.first() {
        Some(NetEvent::Request { method, path, type_, party }) => format!("{method} {path} [{type_}{}]", if party == "third" { ", third-party" } else { "" }),
        _ => "(no request)".into(),
    }
}
fn outcome_of(t: &NetTx) -> String {
    match t.events.get(1) {
        Some(NetEvent::Response { status }) => format!("status {status}"),
        Some(NetEvent::Aborted) => "aborted".into(),
        Some(NetEvent::Failed) => "failed".into(),
        Some(NetEvent::Pending) | None => "pending".into(),
        Some(NetEvent::Request { .. }) => "none".into(),
    }
}

/// Added / removed / changed / same for two transaction lists: multisets of fingerprints; leftovers pair by request key into `changed`.
fn compare_tx(a: &[NetTx], b: &[NetTx]) -> (Vec<Value>, [usize; 4]) {
    fn bucket(l: &[NetTx]) -> BTreeMap<&str, Vec<&NetTx>> {
        let mut m: BTreeMap<&str, Vec<&NetTx>> = BTreeMap::new();
        for t in l {
            m.entry(t.fingerprint.as_str()).or_default().push(t);
        }
        m
    }
    let (ma, mb) = (bucket(a), bucket(b));
    let (mut gone, mut fresh): (Vec<&NetTx>, Vec<&NetTx>) = (vec![], vec![]);
    let mut same = 0;
    for (fp, la) in &ma {
        let nb = mb.get(fp).map_or(0, |l| l.len());
        same += la.len().min(nb);
        gone.extend(la.iter().skip(nb));
    }
    for (fp, lb) in &mb {
        fresh.extend(lb.iter().skip(ma.get(fp).map_or(0, |l| l.len())));
    }
    let mut by_key: BTreeMap<String, Vec<&NetTx>> = BTreeMap::new();
    for t in &fresh {
        by_key.entry(key_of(t)).or_default().push(t);
    }
    let (mut items, mut n) = (vec![], [0usize; 4]);
    // `gone` = requests only ours made (added relative to the reference); what is left of `fresh` = only the reference made (removed)
    for t in gone {
        let k = key_of(t);
        match by_key.get_mut(&k).filter(|l| !l.is_empty()).map(|l| l.remove(0)) {
            Some(p) => {
                n[2] += 1;
                items.push(json!({ "kind": "changed", "key": k, "ours": outcome_of(t), "ref": outcome_of(p) }));
            }
            None => {
                n[0] += 1;
                items.push(json!({ "kind": "added", "key": k, "outcome": outcome_of(t) }));
            }
        }
    }
    for (k, l) in by_key {
        for t in l {
            n[1] += 1;
            items.push(json!({ "kind": "removed", "key": k, "outcome": outcome_of(t) }));
        }
    }
    n[3] = same;
    items.sort_by(|x, y| (x["key"].as_str(), x["kind"].as_str()).cmp(&(y["key"].as_str(), y["kind"].as_str())));
    (items, n)
}

/// The `net` block as transactions: only the failed calls are per-request there (`FAILED <path>`, outcome `status N`).
fn from_failed(n: &Net) -> Vec<NetTx> {
    n.failed
        .iter()
        .map(|f| NetTx {
            events: vec![
                NetEvent::Request { method: "FAILED".into(), path: f.path.clone(), type_: "net".into(), party: "first".into() },
                NetEvent::Response { status: f.status },
            ],
            fingerprint: format!("failed:{}:{}", f.status, f.path),
            parent: None,
        })
        .collect()
}

/// Shapes both sides recorded for the same `METHOD path`, whose digest sets differ.
fn shape_items(a: &[NetShape], b: &[NetShape]) -> Vec<Value> {
    fn idx(l: &[NetShape]) -> BTreeMap<String, BTreeSet<&str>> {
        let mut m: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
        for s in l {
            m.entry(format!("{} {}", s.method, s.path)).or_default().insert(s.digest.as_str());
        }
        m
    }
    let (ia, ib) = (idx(a), idx(b));
    ia.iter()
        .filter_map(|(k, da)| ib.get(k).filter(|db| *db != da).map(|db| json!({ "kind": "shape", "key": k, "ours": da, "ref": db })))
        .collect()
}

fn device_ok(l: &Loaded, device: &str) -> bool {
    l.ev.viewport == device || l.ev.device.as_ref().is_some_and(|d| d.id == device)
}

type Key = (String, String, String);
/// Best capture per (screen, state, viewport): the preferred seat.
fn best<'a>(set: &'a super::ui::UiSet, product: &str, o: &Opts, screen: Option<&str>, state: Option<&str>, device: Option<&str>) -> BTreeMap<Key, &'a Loaded> {
    let mut m: BTreeMap<Key, &Loaded> = BTreeMap::new();
    for l in set.evidence_of(product) {
        let e = &l.ev;
        if e.locale != o.locale || e.theme != o.theme || e.unstable {
            continue;
        }
        if screen.is_some_and(|s| s != e.screen) || state.is_some_and(|s| s != e.state) || device.is_some_and(|d| !device_ok(l, d)) {
            continue;
        }
        let k = (e.screen.clone(), e.state.clone(), e.viewport.clone());
        match m.get(&k) {
            Some(cur) if seat_rank(&cur.ev.seat) <= seat_rank(&e.seat) => {}
            _ => {
                m.insert(k, l);
            }
        }
    }
    m
}

fn comp_line(c: &Option<crate::domain::ui::Completeness>) -> Value {
    match c {
        None => json!({ "status": "not recorded" }),
        Some(c) => json!({ "status": c.status, "dropped": c.dropped_events.total, "truncatedSections": c.truncated_sections }),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn net_diff(root: &Path, max_nodes: usize, ours: &str, refp: &str, screen: Option<&str>, state: Option<&str>, device: Option<&str>, o: &Opts) -> Result<Value> {
    need_token("--ours", ours)?;
    need_token("--ref", refp)?;
    for (w, v) in [("--screen", screen), ("--state", state), ("--device", device)] {
        if let Some(v) = v {
            need_token(w, v)?;
        }
    }
    let set = open(root, max_nodes, Some(&[ours, refp]), EV)?;
    super::ui::need_product_pub(&set, ours)?;
    super::ui::need_product_pub(&set, refp)?;
    let (a, b) = (best(&set, ours, o, screen, state, device), best(&set, refp, o, screen, state, device));
    let shared: Vec<&Key> = a.keys().filter(|k| b.contains_key(*k)).collect();
    if shared.is_empty() {
        let cands: Vec<String> = a.keys().map(|k| format!("{}/{}/{}", k.0, k.1, k.2)).collect();
        return Err(AppError::not_found_with(
            format!("`{ours}` and `{refp}` share no capture for screen {} state {} device {} (a comparison is only within the same screen, state and device)", screen.unwrap_or("any"), state.unwrap_or("any"), device.unwrap_or("any")),
            &format!("{}/{}/{}", screen.unwrap_or("any"), state.unwrap_or("any"), device.unwrap_or("any")),
            cands,
            "list the captures with `comparegraph ui screens <product>` and pass --screen, --state and --device for one both have",
        ));
    }
    let mut lims: Vec<Limitation> = vec![];
    let (mut captures, mut skipped, mut drift_all) = (vec![], vec![], vec![]);
    let (mut tot, mut used_behaviour, mut worst) = ([0usize; 5], 0usize, "unchanged");
    for k in shared {
        let (la, lb) = (a[k], b[k]);
        let label = format!("{}/{}/{}", k.0, k.1, k.2);
        let drift = evidence_drift(&la.ev, &lb.ev);
        if !drift.is_empty() {
            if !o.allow_drift {
                return Err(drift_error(&drift, &format!("the captures of {label}")));
            }
            drift_all.extend(drift);
        }
        let (Some(na), Some(nb)) = (&la.ev.net, &lb.ev.net) else {
            skipped.push(label);
            continue;
        };
        let mut reasons: Vec<String> = vec![];
        // the primary input: net.behaviour on both sides; else the failed calls of the net block
        let (source, ta, tb, norm_ok) = match (&na.behaviour, &nb.behaviour) {
            (Some(x), Some(y)) => {
                used_behaviour += 1;
                for (side, r) in [("ours", x), ("ref", y)] {
                    reasons.extend(r.coverage.reasons.iter().map(|m| format!("{side}: {m}")));
                }
                ("net-behaviour/1", x.transactions.clone(), y.transactions.clone(), x.normalization.sha256 == y.normalization.sha256)
            }
            (x, y) => {
                if x.is_some() != y.is_some() {
                    lims.push(Limitation::new("net_behaviour_one_side", format!("{label}: only one side records net.behaviour; compared on the `net` block (failed calls) instead")));
                }
                ("net", from_failed(na), from_failed(nb), true)
            }
        };
        if !norm_ok {
            reasons.push("the two records were normalised with different committed rules; they are not comparable".into());
        }
        for (side, n) in [("ours", na), ("ref", nb)] {
            match &n.completeness {
                Some(c) if c.truncated() => reasons.push(format!("{side}: capture is {} ({} dropped event(s)); an unchanged claim is not supported", c.status, c.dropped_events.total)),
                None => reasons.push(format!("{side}: no net.completeness recorded (captured before REA-A); completeness unknown")),
                _ => {}
            }
        }
        let (mut items, n) = if norm_ok { compare_tx(&ta, &tb) } else { (vec![], [0; 4]) };
        let shapes = match (&na.shapes, &nb.shapes) {
            (Some(x), Some(y)) if norm_ok => shape_items(x, y),
            _ => vec![],
        };
        let status = if n[0] + n[1] + n[2] + shapes.len() > 0 {
            "changed"
        } else if !reasons.is_empty() {
            "unknown"
        } else {
            "unchanged"
        };
        worst = match (worst, status) {
            ("changed", _) | (_, "changed") => "changed",
            ("unknown", _) | (_, "unknown") => "unknown",
            _ => "unchanged",
        };
        for i in 0..4 {
            tot[i] += n[i];
        }
        tot[4] += shapes.len();
        items.extend(shapes.iter().cloned());
        items.sort_by(|x, y| (x["key"].as_str(), x["kind"].as_str()).cmp(&(y["key"].as_str(), y["kind"].as_str())));
        if items.len() > MAX_ITEMS {
            lims.push(Limitation::new("net_items_capped", format!("{label}: {} differences, the first {MAX_ITEMS} are listed (summary counts all)", items.len())));
        }
        let deltas = if source == "net" {
            json!({ "requests": [na.requests, nb.requests], "bytes": [na.bytes, nb.bytes], "thirdPartyHosts": [na.third_party_hosts, nb.third_party_hosts], "byType": [na.by_type, nb.by_type] })
        } else {
            json!({})
        };
        captures.push(json!({
            "screen": k.0, "state": k.1, "viewport": k.2, "source": source, "status": status,
            "summary": { "added": n[0], "removed": n[1], "changed": n[2], "same": n[3], "shape": shapes.len() },
            "completeness": { "ours": comp_line(&na.completeness), "ref": comp_line(&nb.completeness) },
            "reasons": reasons, "deltas": deltas,
            "items": items.into_iter().take(MAX_ITEMS).collect::<Vec<_>>(),
        }));
    }
    if captures.is_empty() {
        return Err(AppError::not_found_with(
            format!("no shared capture of `{ours}` and `{refp}` records a `net` block: {}", skipped.join(", ")),
            "net",
            skipped,
            "re-capture with the probe (net is recorded since CMP-UI-1e; net.behaviour needs --net-behaviour)",
        ));
    }
    if !skipped.is_empty() {
        lims.push(Limitation::new("net_missing", format!("{} shared capture(s) record no `net` on at least one side and were not compared: {}", skipped.len(), skipped.join(", "))));
    }
    if !drift_all.is_empty() {
        lims.push(drift_limitation(&drift_all));
    }
    if used_behaviour == 0 {
        lims.push(Limitation::new("net_behaviour_absent", "no compared capture carries net.behaviour on both sides: only failed calls and totals (the `net` block) were compared"));
    }
    let source = if used_behaviour == captures.len() { "net-behaviour/1" } else if used_behaviour == 0 { "net" } else { "mixed" };
    let v = json!({
        "ours": ours, "ref": refp, "source": source, "status": worst,
        "normalisation": "the committed rules of net.behaviour.normalization (exact-literal replacements, compared by their sha256); requests are identified by the probe's fingerprint, never recomputed here",
        "summary": { "added": tot[0], "removed": tot[1], "changed": tot[2], "same": tot[3], "shape": tot[4], "captures": captures.len() },
        "captures": captures,
    });
    Ok(set.finish(v, &lims))
}
