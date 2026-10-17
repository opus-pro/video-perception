use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(label: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("video-perception-{label}-{unique}"));
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn extension_list_discovers_valid_manifests_and_reports_invalid_ones() {
    let bin = env!("CARGO_BIN_EXE_video-perception");
    let directory = temp_dir("extension-list");
    std::fs::write(
        directory.join("provider.yaml"),
        r#"contract: video-perception.provider/v1
id: user.local-transcript
version: 0.1.0
description: Existing local transcript tool
locality: local
media_transfer: never
runtime:
  type: command
  command: [existing-transcript-tool]
  protocol: video-perception.provider-stdio/v1
capabilities:
  - id: transcript
    contract: video-perception.transcript/v1
"#,
    )
    .unwrap();
    std::fs::write(directory.join("broken.yaml"), "id: missing-contract\n").unwrap();

    let output = Command::new(bin)
        .args(["extension", "list", "--json"])
        .env("VIDEO_PERCEPTION_EXTENSION_PATH", &directory)
        .env("VIDEO_PERCEPTION_HOME", directory.join("empty-home"))
        .current_dir(&directory)
        .output()
        .expect("Failed to list extensions");

    assert!(
        output.status.success(),
        "discovery should remain inspectable: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["contract"], "video-perception.extensions/v1");
    assert_eq!(report["extensions"][0]["id"], "user.local-transcript");
    assert_eq!(report["errors"].as_array().unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn capabilities_includes_read_only_extension_discovery() {
    let bin = env!("CARGO_BIN_EXE_video-perception");
    let directory = temp_dir("capability-extensions");
    std::fs::write(
        directory.join("strategy.yaml"),
        r#"contract: video-perception.strategy/v1
id: user.quick-scan
version: 0.1.0
description: Quick deterministic scan
plan:
  contract: video-perception.sense-plan/v1
  capabilities:
    - id: scenes
"#,
    )
    .unwrap();

    let output = Command::new(bin)
        .args(["capabilities", "--json"])
        .env("VIDEO_PERCEPTION_EXTENSION_PATH", &directory)
        .env("VIDEO_PERCEPTION_HOME", directory.join("empty-home"))
        .current_dir(&directory)
        .output()
        .expect("Failed to discover capabilities");

    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report["extensions"]["contract"],
        "video-perception.extensions/v1"
    );
    assert_eq!(
        report["extensions"]["extensions"][0]["id"],
        "user.quick-scan"
    );
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn bundled_extension_examples_validate_as_a_set() {
    let bin = env!("CARGO_BIN_EXE_video-perception");
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();

    for relative in [
        "examples/extensions",
        "plugin/strategies",
        "plugin/packs/foundation",
    ] {
        let output = Command::new(bin)
            .args(["extension", "validate"])
            .arg(repo.join(relative))
            .arg("--json")
            .output()
            .unwrap_or_else(|error| panic!("Failed to validate {relative}: {error}"));
        assert!(
            output.status.success(),
            "{relative} should validate: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["valid"], true);
    }
}
