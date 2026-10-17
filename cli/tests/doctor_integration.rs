use std::process::Command;

#[test]
fn doctor_json_output_is_valid() {
    let bin = env!("CARGO_BIN_EXE_video-perception");
    let output = Command::new(bin)
        .args(["doctor", "--json"])
        .env("GEMINI_API_KEY", "doctor-must-ignore-this")
        .output()
        .expect("Failed to run video-perception");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("Doctor --json output is not valid JSON");

    let arr = parsed.as_array().expect("Top-level should be array");
    assert!(
        arr.len() >= 4,
        "Should have at least 4 groups, got {}",
        arr.len()
    );
    for group_name in ["Model runner", "Local model route (optional, user-managed)"] {
        assert!(
            arr.iter().any(|group| group["group"] == group_name),
            "Doctor should distinguish model-layer role: {group_name}"
        );
    }
    assert!(
        !stdout.contains("GEMINI_API_KEY"),
        "Doctor must not inspect or advertise credentials or private routes"
    );

    for group in arr {
        assert!(group.get("group").is_some(), "Each group needs 'group' key");
        let items = group["items"].as_array().expect("items should be array");
        for item in items {
            assert!(item.get("name").is_some(), "Each item needs 'name'");
            assert!(item.get("ok").is_some(), "Each item needs 'ok'");
            let ok = item["ok"].as_bool().unwrap();
            if !ok {
                assert!(
                    item.get("fix").is_some(),
                    "Not-ok item '{}' should have 'fix' key",
                    item["name"]
                );
            }
        }
    }
}

#[test]
fn doctor_table_output_shows_summary() {
    let bin = env!("CARGO_BIN_EXE_video-perception");
    let output = Command::new(bin)
        .args(["doctor"])
        .output()
        .expect("Failed to run video-perception");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("capabilities available"),
        "Table output should show summary line"
    );
    assert!(
        stderr.contains("Core (required)"),
        "Table output should show Core group"
    );
}
