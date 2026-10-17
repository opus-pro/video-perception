use std::path::PathBuf;
use std::process::Command;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

// ── diff: two Observations ──

#[test]
fn test_diff_produces_valid_json() {
    let bin = env!("CARGO_BIN_EXE_video-perception");
    let obs_a = fixtures_dir().join("obs");
    let obs_b = fixtures_dir().join("obs_b");

    let output = Command::new(bin)
        .args([
            "observation",
            "diff",
            obs_a.to_str().unwrap(),
            obs_b.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run diff");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "diff should succeed, stderr: {stderr}"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let result: serde_json::Value =
        serde_json::from_str(&stdout).expect("diff should produce valid JSON");

    assert_eq!(result["mode"], "diff");
    assert!(result["summary"].is_object(), "should have summary");
    assert!(result["scene_diffs"].is_array(), "should have scene_diffs");
}

#[test]
fn test_diff_detects_changes() {
    let bin = env!("CARGO_BIN_EXE_video-perception");
    let obs_a = fixtures_dir().join("obs");
    let obs_b = fixtures_dir().join("obs_b");

    let output = Command::new(bin)
        .args([
            "observation",
            "diff",
            obs_a.to_str().unwrap(),
            obs_b.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run diff");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let summary = &result["summary"];
    let total = summary["total_fields_compared"].as_u64().unwrap_or(0);

    assert!(total > 0, "should compare at least 1 field");
    assert!(
        summary.get("notable_changes").is_none(),
        "consumer policy must not appear in sensor diff"
    );

    // Scene 0: brightness 150.2 → 80.0 is emitted as a raw percentage delta.
    let scene_diffs = result["scene_diffs"].as_array().unwrap();
    assert!(!scene_diffs.is_empty(), "should have scene-level diffs");
}

#[test]
fn test_diff_same_observation() {
    let bin = env!("CARGO_BIN_EXE_video-perception");
    let obs = fixtures_dir().join("obs");

    let output = Command::new(bin)
        .args([
            "observation",
            "diff",
            obs.to_str().unwrap(),
            obs.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run diff");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let total = result["summary"]["total_fields_compared"]
        .as_u64()
        .unwrap_or(999);
    assert_eq!(total, 0, "diff of same Observation should have no deltas");
}

#[test]
fn test_diff_rejects_consumer_owned_fail_policy() {
    let bin = env!("CARGO_BIN_EXE_video-perception");
    let obs_a = fixtures_dir().join("obs");
    let obs_b = fixtures_dir().join("obs_b");

    let output = Command::new(bin)
        .args([
            "observation",
            "diff",
            obs_a.to_str().unwrap(),
            obs_b.to_str().unwrap(),
            "--fail",
            "5",
        ])
        .output()
        .expect("Failed to run diff with consumer policy");

    assert!(
        !output.status.success(),
        "Sensor must reject consumer-owned pass/fail policy"
    );
}
