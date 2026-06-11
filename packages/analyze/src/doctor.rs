//! `matchy doctor` — verify runtime environment (M1.md §5.5).
//!
//! Checks:
//! 1. node on PATH >= 20
//! 2. capture.cjs resolvable
//! 3. capture doctor mode (playwright version, Chromium availability)
//!
//! Prints status table + remediation commands; exit 0 healthy, 1 not.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::orchestrate::{extract_chromium_rev, format_browser_remedy, resolve_capture_script};

struct Check {
    name: &'static str,
    ok: bool,
    detail: String,
    remediation: Option<String>,
}

/// Run all doctor checks and print results.
/// Returns true if all checks passed.
pub fn run_doctor() -> bool {
    let mut checks: Vec<Check> = Vec::new();

    // 1. Node.js version >= 20
    checks.push(check_node());

    // 2. capture.cjs resolvable
    let capture_check = check_capture_script();
    let capture_ok = capture_check.ok;
    let capture_path = if capture_ok {
        Some(capture_check.detail.clone())
    } else {
        None
    };
    checks.push(capture_check);

    // 3. Playwright + Chromium (via capture doctor mode)
    if let Some(path) = capture_path {
        checks.extend(check_capture_doctor(&path));
    } else {
        checks.push(Check {
            name: "playwright",
            ok: false,
            detail: "skipped (capture.cjs not found)".to_string(),
            remediation: None,
        });
        checks.push(Check {
            name: "chromium",
            ok: false,
            detail: "skipped (capture.cjs not found)".to_string(),
            remediation: None,
        });
    }

    // Print table
    println!("\n{:<20} {:<10} Detail", "Check", "Status");
    println!("{}", "-".repeat(72));
    for check in &checks {
        let status = if check.ok { "OK" } else { "FAIL" };
        println!("{:<20} {:<10} {}", check.name, status, check.detail);
        if let Some(rem) = &check.remediation {
            println!("{:<20} {:<10} Remedy: {}", "", "", rem);
        }
    }
    println!();

    let all_ok = checks.iter().all(|c| c.ok);
    if all_ok {
        println!("All checks passed. This machine is healthy for matchy.");
    } else {
        eprintln!("One or more checks failed. Follow the remediation steps above.");
    }
    all_ok
}

fn check_node() -> Check {
    match Command::new("node").arg("--version").output() {
        Err(e) => Check {
            name: "node.js",
            ok: false,
            detail: format!("not found: {}", e),
            remediation: Some("Install Node.js >= 20 from https://nodejs.org".to_string()),
        },
        Ok(out) => {
            let version_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
            // Parse vMAJOR.MINOR.PATCH
            let major = parse_node_major(&version_str);
            if major >= 20 {
                Check {
                    name: "node.js",
                    ok: true,
                    detail: version_str,
                    remediation: None,
                }
            } else {
                Check {
                    name: "node.js",
                    ok: false,
                    detail: format!("{} (need >= 20)", version_str),
                    remediation: Some(
                        "Upgrade Node.js to >= 20. Use nvm: `nvm install 20 && nvm use 20`"
                            .to_string(),
                    ),
                }
            }
        }
    }
}

fn parse_node_major(version: &str) -> u32 {
    // format: "v20.1.2" or "20.1.2"
    let stripped = version.trim_start_matches('v');
    stripped
        .split('.')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn check_capture_script() -> Check {
    match resolve_capture_script() {
        Ok(path) => Check {
            name: "capture.cjs",
            ok: true,
            detail: path.display().to_string(),
            remediation: None,
        },
        Err(e) => Check {
            name: "capture.cjs",
            ok: false,
            detail: format!("not found: {}", e),
            remediation: Some(
                "Build capture: `cd packages/capture && npm ci && npm run build`".to_string(),
            ),
        },
    }
}

/// Parsed fields from capture's doctor-mode JSON response.
/// All fields are optional so old builds that omit the new fields still work.
struct DoctorResponse {
    playwright_ok: bool,
    playwright_version: String,
    chromium_ok_raw: bool, // from chromium.ok in the JSON
    chromium_version: String,
    chromium_executable_path: Option<String>, // new: may be absent in old builds
    chromium_exists: Option<bool>,            // new: may be absent in old builds
    browsers_path: Option<String>,            // new: may be absent in old builds
}

fn parse_doctor_response(value: &serde_json::Value) -> DoctorResponse {
    let playwright_version = value
        .get("playwright")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let playwright_ok = value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);

    let chromium = value.get("chromium");

    let chromium_ok_raw = chromium
        .and_then(|c| c.get("ok"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let chromium_version = chromium
        .and_then(|c| c.get("version"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    // New fields — tolerate absence for old capture.cjs builds
    let chromium_executable_path = chromium
        .and_then(|c| c.get("executablePath"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let chromium_exists = chromium
        .and_then(|c| c.get("exists"))
        .and_then(|v| v.as_bool());

    let browsers_path = value
        .get("browsersPath")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    DoctorResponse {
        playwright_ok,
        playwright_version,
        chromium_ok_raw,
        chromium_version,
        chromium_executable_path,
        chromium_exists,
        browsers_path,
    }
}

fn check_capture_doctor(capture_path: &str) -> Vec<Check> {
    let config_json = r#"{"mode":"doctor"}"#;

    let result = Command::new("node")
        .arg(capture_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    let mut child = match result {
        Err(e) => {
            return vec![
                Check {
                    name: "playwright",
                    ok: false,
                    detail: format!("failed to spawn node: {}", e),
                    remediation: Some("Ensure node is on PATH".to_string()),
                },
                Check {
                    name: "chromium",
                    ok: false,
                    detail: "skipped".to_string(),
                    remediation: None,
                },
            ]
        }
        Ok(c) => c,
    };

    {
        use std::io::Write;
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(config_json.as_bytes());
        }
    }

    let output = match child.wait_with_output() {
        Err(e) => {
            return vec![
                Check {
                    name: "playwright",
                    ok: false,
                    detail: format!("capture doctor failed: {}", e),
                    remediation: Some(
                        "Run `cd packages/capture && npm ci && npx playwright install chromium`"
                            .to_string(),
                    ),
                },
                Check {
                    name: "chromium",
                    ok: false,
                    detail: "skipped".to_string(),
                    remediation: None,
                },
            ]
        }
        Ok(o) => o,
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.lines().next().unwrap_or("");

    if line.is_empty() {
        return vec![
            Check {
                name: "playwright",
                ok: false,
                detail: "capture doctor returned no output".to_string(),
                remediation: Some(
                    "Run `cd packages/capture && npm ci && npx playwright install chromium`"
                        .to_string(),
                ),
            },
            Check {
                name: "chromium",
                ok: false,
                detail: "skipped".to_string(),
                remediation: None,
            },
        ];
    }

    // Parse doctor response
    let value: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            return vec![
                Check {
                    name: "playwright",
                    ok: false,
                    detail: format!("capture doctor parse error: {} (raw: {})", e, line),
                    remediation: None,
                },
                Check {
                    name: "chromium",
                    ok: false,
                    detail: "skipped".to_string(),
                    remediation: None,
                },
            ]
        }
    };

    let resp = parse_doctor_response(&value);

    // Determine final chromium ok:
    // If exists field is present, ok requires both raw ok AND exists==true.
    // If exists is absent (old build), fall back to raw ok.
    let chromium_ok = match resp.chromium_exists {
        Some(exists) => resp.chromium_ok_raw && exists,
        None => resp.chromium_ok_raw,
    };

    // Build chromium detail and remediation
    let browsers_path_display = resp
        .browsers_path
        .as_deref()
        .unwrap_or("(not set — Playwright is using ~/.cache/ms-playwright)");

    let chromium_detail = if chromium_ok {
        format!(
            "build {}  browsers: {}",
            resp.chromium_version, browsers_path_display
        )
    } else {
        format!("build {}  (FAIL)", resp.chromium_version)
    };

    let chromium_remediation = if chromium_ok {
        None
    } else {
        // Construct targeted remedy
        let executable_line = resp
            .chromium_executable_path
            .as_deref()
            .map(|p| format!("  Expected executable: {}", p))
            .unwrap_or_default();

        // Try to derive repo root from capture_path (resolve_capture_script recorded it,
        // but we can also find it by walking ancestors of capture_path).
        let repo_root = find_repo_root_from_capture(capture_path);

        let remedy_block = match repo_root.as_deref() {
            Some(root) => {
                // Build the standard install command
                let rev = resp
                    .chromium_executable_path
                    .as_deref()
                    .and_then(extract_chromium_rev)
                    .unwrap_or_else(|| "?".to_string());
                format!(
                    "Chromium build {} not found.\n  Current browsers path: {}\n{}\n  Install:\n    cd {}/packages/capture && PLAYWRIGHT_BROWSERS_PATH={}/.pw-browsers npx playwright install chromium\n  Then run matchy with:\n    export PLAYWRIGHT_BROWSERS_PATH={}/.pw-browsers",
                    rev,
                    browsers_path_display,
                    executable_line,
                    root.display(),
                    root.display(),
                    root.display()
                )
            }
            None => {
                let missing_path = resp
                    .chromium_executable_path
                    .as_deref()
                    .unwrap_or("<unknown>");
                format_browser_remedy(missing_path, None)
            }
        };
        Some(remedy_block)
    };

    vec![
        Check {
            name: "playwright",
            ok: resp.playwright_ok,
            detail: format!("v{}", resp.playwright_version),
            remediation: if resp.playwright_ok {
                None
            } else {
                Some("Run `cd packages/capture && npm ci`".to_string())
            },
        },
        Check {
            name: "chromium",
            ok: chromium_ok,
            detail: chromium_detail,
            remediation: chromium_remediation,
        },
    ]
}

/// Walk ancestors of `capture_path` to find the repo root
/// (the ancestor that has a `packages/capture` subdirectory).
fn find_repo_root_from_capture(capture_path: &str) -> Option<PathBuf> {
    let p = std::path::Path::new(capture_path);
    p.ancestors()
        .find(|a| a.join("packages/capture").is_dir())
        .map(|a| a.to_path_buf())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_doctor_response_old_build_no_new_fields() {
        // Old capture.cjs that doesn't return executablePath / exists / browsersPath
        let value = json!({
            "ok": true,
            "node": "v24.0.0",
            "playwright": "1.60.0",
            "chromium": { "ok": true, "version": "chromium-1223" }
        });
        let resp = parse_doctor_response(&value);
        assert!(resp.playwright_ok);
        assert!(resp.chromium_ok_raw);
        assert_eq!(resp.chromium_version, "chromium-1223");
        assert!(resp.chromium_executable_path.is_none());
        assert!(resp.chromium_exists.is_none());
        assert!(resp.browsers_path.is_none());
    }

    #[test]
    fn parse_doctor_response_new_fields_ok() {
        let value = json!({
            "ok": true,
            "node": "v24.0.0",
            "playwright": "1.60.0",
            "chromium": {
                "ok": true,
                "version": "chromium-1223",
                "executablePath": "/repo/.pw-browsers/chromium_headless_shell-1223/chrome-linux/chrome",
                "exists": true
            },
            "browsersPath": "/repo/.pw-browsers"
        });
        let resp = parse_doctor_response(&value);
        assert!(resp.playwright_ok);
        assert!(resp.chromium_ok_raw);
        assert_eq!(
            resp.chromium_executable_path.as_deref(),
            Some("/repo/.pw-browsers/chromium_headless_shell-1223/chrome-linux/chrome")
        );
        assert_eq!(resp.chromium_exists, Some(true));
        assert_eq!(resp.browsers_path.as_deref(), Some("/repo/.pw-browsers"));
    }

    #[test]
    fn parse_doctor_response_exists_false_overrides_ok() {
        // chromium.ok is true (launch hasn't been verified), but exists is false
        let value = json!({
            "ok": false,
            "node": "v24.0.0",
            "playwright": "1.60.0",
            "chromium": {
                "ok": false,
                "version": "",
                "executablePath": "/home/user/.cache/ms-playwright/chromium_headless_shell-1223/chrome-linux/chrome",
                "exists": false
            },
            "browsersPath": null
        });
        let resp = parse_doctor_response(&value);
        // When exists == false, chromium_ok should be false even if raw was true
        let chromium_ok = match resp.chromium_exists {
            Some(exists) => resp.chromium_ok_raw && exists,
            None => resp.chromium_ok_raw,
        };
        assert!(!chromium_ok);
        assert_eq!(resp.chromium_exists, Some(false));
    }

    #[test]
    fn parse_doctor_response_browsers_path_null() {
        let value = json!({
            "ok": false,
            "node": "v24.0.0",
            "playwright": "1.60.0",
            "chromium": { "ok": false, "version": "" },
            "browsersPath": null
        });
        let resp = parse_doctor_response(&value);
        // null browsersPath should be treated as absent
        assert!(resp.browsers_path.is_none());
    }
}
