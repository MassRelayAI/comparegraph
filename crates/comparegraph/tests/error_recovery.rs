//! REA-CG: every error code carries a machine-readable `recovery` ({kind, operation?, retryable}); the catalog lists them all.
use assert_cmd::Command;
use clap::CommandFactory;
use comparegraph::application::catalog::{recovery, ERROR_CODES, RECOVERY, RECOVERY_KINDS};
use serde_json::Value;

#[test]
fn every_error_code_has_one_recovery_of_a_known_kind_naming_a_real_verb() {
    let subs: Vec<String> = comparegraph::adapters::cli::Cli::command().get_subcommands().map(|c| c.get_name().to_string()).collect();
    for code in ERROR_CODES {
        let rows: Vec<_> = RECOVERY.iter().filter(|r| r.0 == *code).collect();
        assert_eq!(rows.len(), 1, "{code} needs exactly one RECOVERY row");
        let (_, kind, op, _) = rows[0];
        assert!(RECOVERY_KINDS.contains(kind), "{code}: unknown kind {kind}");
        if !op.is_empty() {
            let verb = op.split(' ').next().unwrap();
            assert!(subs.iter().any(|s| s == verb), "{code}: `{op}` is not a comparegraph subcommand");
        }
        let v = recovery(code);
        assert_eq!(v["kind"], *kind);
    }
    assert_eq!(RECOVERY.len(), ERROR_CODES.len(), "a recovery row without a code");
}

#[test]
fn a_real_error_envelope_and_the_catalog_carry_it() {
    let d = tempfile::tempdir().unwrap();
    let out = Command::cargo_bin("comparegraph").unwrap().current_dir(d.path()).args(["show", "nope", "nope"]).output().unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], false, "{v}");
    let code = v["error"]["code"].as_str().unwrap();
    assert_eq!(v["error"]["recovery"], recovery(code), "{v}");
    let cat = Command::cargo_bin("comparegraph").unwrap().arg("catalog").output().unwrap();
    let c: Value = serde_json::from_slice(&cat.stdout).unwrap();
    assert_eq!(c["error_recovery"]["store_busy"]["retryable"], true);
    assert_eq!(c["error_recovery"].as_object().unwrap().len(), ERROR_CODES.len());
}
