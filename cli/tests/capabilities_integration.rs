use std::process::Command;

#[test]
fn capabilities_json_exposes_stable_read_only_contract() {
    let bin = env!("CARGO_BIN_EXE_video-perception");
    let output = Command::new(bin)
        .args(["capabilities", "--json"])
        .output()
        .expect("Failed to run capabilities --json");

    assert!(
        output.status.success(),
        "capabilities should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("capabilities --json should emit valid JSON");

    assert_eq!(report["contract"], "video-perception.capabilities/v1");
    assert!(report["device"].is_object());
    assert!(
        report["providers"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
    );
    assert!(
        report["local_models"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
    );
    assert_eq!(
        report["extensions"]["contract"],
        "video-perception.extensions/v1"
    );
    assert_eq!(
        report["guarantees"][0],
        "Discovery performs no downloads or configuration writes."
    );

    let encoded = report.to_string();
    assert!(!encoded.contains("acme-consumer"));
    assert!(!encoded.contains("consumer_profile"));
}

#[test]
fn setup_json_is_an_advisory_plan_with_provider_environment() {
    let bin = env!("CARGO_BIN_EXE_video-perception");
    let output = Command::new(bin)
        .args(["setup", "--profile", "local-model", "--json"])
        .output()
        .expect("Failed to run setup --json");

    assert!(output.status.success());
    let plan: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("setup --json should emit valid JSON");
    assert_eq!(plan["contract"], "video-perception.setup-plan/v1");
    assert_eq!(plan["mutates_system"], false);
    assert_eq!(plan["profile"]["id"], "local-model");
    assert_eq!(
        plan["profile"]["environment"]["VIDEO_PERCEPTION_TRANSCRIPT_POLICY"],
        "local-only"
    );
    assert_eq!(
        plan["profile"]["environment"]["VIDEO_PERCEPTION_VLM_BACKEND"],
        "ollama"
    );
}

#[test]
fn setup_does_not_write_project_configuration() {
    let bin = env!("CARGO_BIN_EXE_video-perception");
    let temp = std::env::temp_dir().join(format!(
        "video-perception-setup-read-only-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();

    let output = Command::new(bin)
        .args(["setup", "--profile", "basic"])
        .current_dir(&temp)
        .output()
        .expect("Failed to run setup");

    assert!(output.status.success());
    assert!(!temp.join("video-perception.yaml").exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("changed nothing on the system"));
    let _ = std::fs::remove_dir_all(&temp);
}

#[test]
fn setup_rejects_unknown_profile() {
    let bin = env!("CARGO_BIN_EXE_video-perception");
    let output = Command::new(bin)
        .args(["setup", "--profile", "consumer-specific"])
        .output()
        .expect("Failed to run setup");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Unknown setup profile"));
    assert!(stderr.contains("mac-native"));
    assert!(stderr.contains("basic"));
    assert!(stderr.contains("local-model"));
}

#[test]
fn plugin_manifests_are_valid_and_versioned_with_cli() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    for relative in [
        "plugin/.claude-plugin/plugin.json",
        "plugin/.codex-plugin/plugin.json",
    ] {
        let manifest: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(repo.join(relative))
                .unwrap_or_else(|error| panic!("{relative} should exist: {error}")),
        )
        .unwrap_or_else(|error| panic!("{relative} should be valid JSON: {error}"));
        assert_eq!(manifest["name"], "video-perception");
        assert_eq!(manifest["version"], env!("CARGO_PKG_VERSION"));
    }
}

#[cfg(unix)]
#[test]
fn discovery_does_not_invoke_network_client() {
    use std::os::unix::fs::PermissionsExt;

    let bin = env!("CARGO_BIN_EXE_video-perception");
    let temp = std::env::temp_dir().join(format!(
        "video-perception-no-network-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();
    let marker = temp.join("curl-was-invoked");
    let curl = temp.join("curl");
    std::fs::write(
        &curl,
        "#!/bin/sh\ntouch \"$VIDEO_PERCEPTION_CURL_MARKER\"\nexit 99\n",
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&curl).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&curl, permissions).unwrap();

    let inherited_path = std::env::var("PATH").unwrap_or_default();
    let test_path = format!("{}:{inherited_path}", temp.display());
    let output = Command::new(bin)
        .args(["capabilities", "--json"])
        .env("PATH", test_path)
        .env("VIDEO_PERCEPTION_CURL_MARKER", &marker)
        .output()
        .expect("Failed to run capabilities with a guarded curl");

    assert!(output.status.success());
    assert!(!marker.exists(), "discovery must never invoke curl");
    let _ = std::fs::remove_dir_all(&temp);
}

#[test]
fn capabilities_human_output_explains_local_task_routes() {
    let output = Command::new(env!("CARGO_BIN_EXE_video-perception"))
        .arg("capabilities")
        .env_remove("VIDEO_PERCEPTION_TRANSCRIPT_PROVIDER")
        .env_remove("VIDEO_PERCEPTION_TRANSCRIPT_POLICY")
        .env_remove("VIDEO_PERCEPTION_VLM_BACKEND")
        .output()
        .expect("run human-readable capabilities");
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);

    for expected in [
        "Transcript          local providers only (auto selection)",
        "Visual semantics    user-managed local model (Ollama)",
        "Remote media        disabled by current routing",
        "Provider live       not-probed; run video-perception doctor",
        "System native",
        "Local model",
    ] {
        assert!(
            stderr.contains(expected),
            "missing `{expected}` in:\n{stderr}"
        );
    }
}

#[test]
fn setup_human_output_compares_current_and_planned_routes_before_consent() {
    let output = Command::new(env!("CARGO_BIN_EXE_video-perception"))
        .args(["setup", "--profile", "local-model"])
        .env_remove("VIDEO_PERCEPTION_TRANSCRIPT_PROVIDER")
        .env_remove("VIDEO_PERCEPTION_TRANSCRIPT_POLICY")
        .env_remove("VIDEO_PERCEPTION_VLM_BACKEND")
        .output()
        .expect("run human-readable setup");
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);

    for expected in [
        "Current task routes (built-in-defaults)",
        "Planned task routes (advisory, not active yet)",
        "Remote media        disabled by this routing plan",
        "Provider live       not checked — run video-perception doctor",
        "apply only after accepting the privacy choice",
        "changed nothing on the system or in the current shell",
    ] {
        assert!(
            stderr.contains(expected),
            "missing `{expected}` in:\n{stderr}"
        );
    }
}

#[test]
fn setup_json_includes_current_routing_without_live_network_checks() {
    let output = Command::new(env!("CARGO_BIN_EXE_video-perception"))
        .args(["setup", "--profile", "basic", "--json"])
        .env("VIDEO_PERCEPTION_TRANSCRIPT_POLICY", "local-only")
        .env("VIDEO_PERCEPTION_VLM_BACKEND", "ollama")
        .output()
        .expect("run setup JSON with local overrides");
    assert!(output.status.success());
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(plan["current_routing"]["transcript_policy"], "local-only");
    assert_eq!(plan["current_routing"]["semantic_provider"], "ollama");
    assert_eq!(plan["current_routing"]["remote_media_possible"], false);
    assert_eq!(plan["mutates_system"], false);
}
