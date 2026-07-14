//! Integration tests for `matchy --version` / `-V` build provenance (U1 of
//! docs/plans/2026-07-10-001-feat-version-build-provenance-plan.md).
//!
//! Follows the `packages/analyze/tests/analyze_cli.rs` convention: drive the
//! compiled binary via `Command::new(env!("CARGO_BIN_EXE_matchy"))`. No regex
//! crate is available — plain string parsing only. Assertions target the
//! FORMAT/shape, not specific SHA or dirty values, since the tree may be
//! dirty during development.

use std::path::PathBuf;
use std::process::Command;

/// Path to the matchy binary (set by Cargo for integration tests).
fn matchy_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_matchy"))
}

/// Run `matchy <flag>` and return trimmed stdout.
fn run_version(flag: &str) -> String {
    let output = Command::new(matchy_bin())
        .arg(flag)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn matchy {}: {}", flag, e));
    assert!(
        output.status.success(),
        "matchy {} must exit 0 (status: {:?})",
        flag,
        output.status
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|e| panic!("matchy {} stdout was not valid utf8: {}", flag, e))
        .trim()
        .to_string()
}

#[test]
fn version_has_provenance_shape() {
    let stdout = run_version("--version");

    assert!(
        stdout.starts_with("matchy "),
        "--version output must start with 'matchy ', got: {:?}",
        stdout
    );
    let rest = stdout.strip_prefix("matchy ").unwrap();

    // rest is now "<semver> (<inner>)"
    assert!(
        rest.chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false),
        "semver portion must start with an ascii digit, got: {:?}",
        rest
    );

    let open_paren = rest.find(" (").unwrap_or_else(|| {
        panic!(
            "expected ' (' separating semver from provenance, got: {:?}",
            rest
        )
    });
    assert!(
        rest.ends_with(')'),
        "--version output must end with ')', got: {:?}",
        stdout
    );
    let inner = &rest[open_paren + 2..rest.len() - 1];

    if inner == "unknown" {
        // R4 fallback shape — nothing further to check.
        return;
    }

    assert!(
        inner.contains(", dirty="),
        "provenance inner must contain ', dirty=', got: {:?}",
        inner
    );
    let (left, dirty_val) = inner.split_once(", dirty=").unwrap();
    assert!(
        dirty_val == "true" || dirty_val == "false",
        "dirty flag must be exactly 'true' or 'false', got: {:?}",
        dirty_val
    );

    let (sha, ts) = left
        .split_once(' ')
        .unwrap_or_else(|| panic!("expected '<sha> <ts>' in provenance, got: {:?}", left));
    assert!(
        sha.len() >= 7 && sha.chars().all(|c| c.is_ascii_hexdigit()),
        "sha must be >= 7 ascii hexdigits, got: {:?}",
        sha
    );
    assert!(
        ts.contains('T') && ts.ends_with('Z'),
        "timestamp must contain 'T' and end with 'Z', got: {:?}",
        ts
    );
}

#[test]
fn version_sha_matches_git_head() {
    let git_output = match Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            eprintln!("skip: git binary not runnable in this test env: {}", e);
            return;
        }
    };
    if !git_output.status.success() {
        eprintln!(
            "skip: `git rev-parse --short HEAD` failed (non-git build env): {:?}",
            git_output.status
        );
        return;
    }
    let head_sha = match String::from_utf8(git_output.stdout) {
        Ok(s) => s.trim().to_string(),
        Err(e) => {
            eprintln!("skip: git output not valid utf8: {}", e);
            return;
        }
    };

    let stdout = run_version("--version");
    if stdout.contains("(unknown)") {
        eprintln!("skip: matchy was built without git provenance ((unknown) fallback)");
        return;
    }

    assert!(
        stdout.contains(&head_sha),
        "--version output {:?} must contain HEAD short sha {:?}",
        stdout,
        head_sha
    );
}

#[test]
fn short_and_long_version_match() {
    let short = run_version("-V");
    let long = run_version("--version");
    assert_eq!(
        short, long,
        "-V and --version must print the identical provenance string"
    );
}
