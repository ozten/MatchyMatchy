//! Cargo build script: embeds git provenance (short SHA, committer UTC
//! timestamp, dirty flag) into the `matchy` binary via `MATCHY_BUILD_INFO`,
//! consumed by `src/bin/matchy.rs`'s `--version` string.
//!
//! Fail-soft by design (spec: docs/plans/2026-07-10-001-feat-version-build-
//! provenance-plan.md, R4): any git failure (missing binary, not a repo,
//! dubious ownership, non-zero exit, non-utf8 output) degrades to the literal
//! `unknown` marker and the build still succeeds. The whole provenance string
//! is composed atomically — a partial failure never leaks a partially-filled
//! string.

use std::process::Command;

fn main() {
    let info = build_info().unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=MATCHY_BUILD_INFO={}", info);

    // Re-run this script when git HEAD (or the ref it points at) moves, so a
    // commit-only change (no Rust source edit) still refreshes the embedded
    // SHA (R3). Only emit these when git itself is available and the paths
    // actually exist — an emitted-but-missing path would make cargo re-run
    // the script on every single build instead of only on git changes.
    if let Some(git_dir) = run_git(&["rev-parse", "--absolute-git-dir"]) {
        let head_path = format!("{}/HEAD", git_dir);
        if std::path::Path::new(&head_path).exists() {
            println!("cargo:rerun-if-changed={}", head_path);
        }

        // If HEAD is a symbolic ref ("ref: refs/heads/main"), also watch the
        // resolved ref file — that's what actually moves on a normal commit.
        if let Ok(head_contents) = std::fs::read_to_string(&head_path) {
            if let Some(refname) = head_contents.trim().strip_prefix("ref: ") {
                let ref_path = format!("{}/{}", git_dir, refname);
                if std::path::Path::new(&ref_path).exists() {
                    println!("cargo:rerun-if-changed={}", ref_path);
                }
            }
        }

        let packed_refs_path = format!("{}/packed-refs", git_dir);
        if std::path::Path::new(&packed_refs_path).exists() {
            println!("cargo:rerun-if-changed={}", packed_refs_path);
        }
    }
}

/// Compose the provenance string, or `None` if any of its three pieces can't
/// be obtained (atomic: never emit a partially-filled string).
fn build_info() -> Option<String> {
    let sha = run_git(&["rev-parse", "--short", "HEAD"])?;
    let ts = run_git_with_env(
        &[
            "show",
            "-s",
            "--date=format-local:%Y-%m-%dT%H:%M:%SZ",
            "--format=%cd",
            "HEAD",
        ],
        &[("TZ", "UTC")],
    )?;
    let status = run_git(&["status", "--porcelain", "--untracked-files=no"])?;
    let dirty = !status.is_empty();
    Some(format!("{} {}, dirty={}", sha, ts, dirty))
}

/// Run `git <args>` with no extra env, returning trimmed stdout on success.
fn run_git(args: &[&str]) -> Option<String> {
    run_git_with_env(args, &[])
}

/// Run `git <args>` with extra env vars set on the child, returning trimmed
/// stdout only if the process spawned, exited successfully, and its stdout is
/// valid UTF-8. Any failure short-circuits to `None` (caller falls back to
/// the `unknown` marker).
fn run_git_with_env(args: &[&str], env: &[(&str, &str)]) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    Some(stdout.trim().to_string())
}
