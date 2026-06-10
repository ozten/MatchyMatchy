//! `matchy doctor` — verify runtime environment (M1.md §5.5).
//!
//! Checks:
//! 1. node on PATH >= 20
//! 2. capture.cjs resolvable
//! 3. capture doctor mode (playwright version, Chromium availability)
//!
//! Prints status table + remediation commands; exit 0 healthy, 1 not.

use std::process::{Command, Stdio};

use crate::orchestrate::resolve_capture_script;

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

    let playwright_version = value
        .get("playwright")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let playwright_ok = value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);

    let chromium_ok = value
        .get("chromium")
        .and_then(|c| c.get("ok"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let chromium_version = value
        .get("chromium")
        .and_then(|c| c.get("version"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    vec![
        Check {
            name: "playwright",
            ok: playwright_ok,
            detail: format!("v{}", playwright_version),
            remediation: if playwright_ok {
                None
            } else {
                Some(
                    "Run `cd packages/capture && npm ci`".to_string(),
                )
            },
        },
        Check {
            name: "chromium",
            ok: chromium_ok,
            detail: format!("build {}", chromium_version),
            remediation: if chromium_ok {
                None
            } else {
                Some("Run `npx playwright install chromium`".to_string())
            },
        },
    ]
}
