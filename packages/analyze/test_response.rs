use serde::{Deserialize};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(untagged)]
pub enum CaptureResponse {
    Ok { ok: bool, bundle_path: String },
    Err { ok: bool, error: CaptureError },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureError {
    pub code: String,
    pub message: String,
}

fn main() {
    let json = r#"{"ok":true,"bundlePath":"/tmp/test-capture/desktop/old.bundle.json"}"#;
    match serde_json::from_str::<CaptureResponse>(json) {
        Ok(resp) => println!("Parsed: {:?}", resp),
        Err(e) => println!("Error: {}", e),
    }
}
