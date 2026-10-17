use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn unique_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "video-perception-{label}-{}-{nanos}",
        std::process::id()
    ))
}

fn make_video(root: &Path) -> PathBuf {
    std::fs::create_dir_all(root).unwrap();
    let video = root.join("fixture.mp4");
    let output = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=160x90:rate=8",
            "-t",
            "1",
            "-pix_fmt",
            "yuv420p",
            video.to_str().unwrap(),
        ])
        .output()
        .expect("ffmpeg is a required Video Perception dependency");
    assert!(
        output.status.success(),
        "create fixture video: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    video
}

fn observe(video: &Path, output_dir: &Path, extra: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_video-perception"));
    command
        .arg("observe")
        .arg(video)
        .args(extra)
        .arg("--out")
        .arg(output_dir);
    command.output().expect("run observe")
}

#[test]
fn default_observation_is_machine_data_without_media_or_rendering() {
    let root = unique_dir("artifact-boundary");
    let video = make_video(&root);
    let observation = root.join("observation");
    let output = observe(&video, &observation, &["--capabilities", "probe,scenes"]);
    assert!(
        output.status.success(),
        "observe should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    for required in ["manifest.json", "dashboard.json", "scenes.jsonl"] {
        assert!(observation.join(required).is_file(), "missing {required}");
    }
    for rendered in ["index.html", "report.md", "README.md"] {
        assert!(
            !observation.join(rendered).exists(),
            "observe created {rendered}"
        );
    }
    assert!(
        !observation
            .join("assets")
            .join(video.file_name().unwrap())
            .exists(),
        "observe must not copy source media"
    );

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(observation.join("manifest.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["contract"], "video-perception.observation/v1");
    assert_eq!(
        manifest["sense_plan"]["contract"],
        "video-perception.sense-plan/v1"
    );
    assert!(manifest.get("rules").is_none());
    assert!(manifest.get("profile").is_none());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn scene_technical_options_are_executed_and_preserved_in_provenance() {
    let root = unique_dir("scene-options");
    let video = make_video(&root);
    let plan = root.join("plan.json");
    std::fs::write(
        &plan,
        r#"{
          "contract": "video-perception.sense-plan/v1",
          "capabilities": [{
            "id": "scenes",
            "options": {"threshold": 0.15, "merge_ms": 321, "merge": false}
          }]
        }"#,
    )
    .unwrap();
    let observation = root.join("observation");
    let output = observe(&video, &observation, &["--plan", plan.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "observe with scene options: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(observation.join("manifest.json")).unwrap())
            .unwrap();
    assert_eq!(
        manifest["sense_plan"]["capabilities"][0]["options"]["threshold"],
        0.15
    );
    assert_eq!(
        manifest["sense_plan"]["capabilities"][0]["options"]["merge_ms"],
        321
    );
    assert_eq!(
        manifest["sense_plan"]["capabilities"][0]["options"]["merge"],
        false
    );

    let scenes: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(observation.join("signals/scenes.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(scenes["merge_threshold_ms"], 321);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn optional_provider_failure_returns_partial_observation_with_provenance() {
    let root = unique_dir("optional-failure");
    let video = make_video(&root);
    let plan = root.join("plan.json");
    std::fs::write(
        &plan,
        r#"{
          "contract": "video-perception.sense-plan/v1",
          "capabilities": [{
            "id": "transcript",
            "required": false,
            "provider": "whisper-cpp",
            "policy": "local-only"
          }]
        }"#,
    )
    .unwrap();
    let observation = root.join("observation");
    let output = Command::new(env!("CARGO_BIN_EXE_video-perception"))
        .args([
            "observe",
            video.to_str().unwrap(),
            "--plan",
            plan.to_str().unwrap(),
            "--out",
        ])
        .arg(&observation)
        .env_remove("WHISPER_CPP_BIN")
        .env_remove("WHISPER_CPP_MODEL")
        .output()
        .expect("run optional transcript plan");
    assert!(
        output.status.success(),
        "optional failure should return partial evidence: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("1 optional failed"),
        "missing optional-failure summary: {stderr}"
    );
    assert!(
        stderr.contains("partial observation: usable evidence was saved"),
        "missing partial-observation guidance: {stderr}"
    );

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(observation.join("manifest.json")).unwrap())
            .unwrap();
    let transcript = manifest["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|capability| capability["id"] == "transcript")
        .unwrap();
    assert_eq!(transcript["requested"], true);
    assert_eq!(transcript["required"], false);
    assert_eq!(transcript["status"], "failed");
    let error = transcript["error"].as_str().unwrap();
    assert!(
        error.contains("whisper-cpp"),
        "missing attempted provider: {error}"
    );
    assert!(
        error.contains("unavailable"),
        "missing provider outcome: {error}"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn required_provider_failure_is_nonzero() {
    let root = unique_dir("required-failure");
    let video = make_video(&root);
    let plan = root.join("plan.json");
    std::fs::write(
        &plan,
        r#"{
          "contract": "video-perception.sense-plan/v1",
          "capabilities": [{
            "id": "transcript",
            "required": true,
            "provider": "whisper-cpp",
            "policy": "local-only"
          }]
        }"#,
    )
    .unwrap();
    let observation = root.join("observation");
    let output = Command::new(env!("CARGO_BIN_EXE_video-perception"))
        .args([
            "observe",
            video.to_str().unwrap(),
            "--plan",
            plan.to_str().unwrap(),
            "--out",
        ])
        .arg(&observation)
        .env_remove("WHISPER_CPP_BIN")
        .env_remove("WHISPER_CPP_MODEL")
        .output()
        .expect("run required transcript plan");
    assert!(
        !output.status.success(),
        "required failure must fail observe"
    );
    assert!(observation.join("signals/execution.json").is_file());
    assert!(!observation.join("manifest.json").exists());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn ambient_project_config_does_not_change_explicit_request() {
    let root = unique_dir("ambient-config");
    let video = make_video(&root);
    std::fs::write(
        root.join("video-perception.yaml"),
        "sensors:\n  - definitely-not-a-capability\nvlm:\n  backend: invalid\n",
    )
    .unwrap();
    let observation = root.join("observation");
    let output = Command::new(env!("CARGO_BIN_EXE_video-perception"))
        .args([
            "observe",
            video.to_str().unwrap(),
            "--capabilities",
            "probe,scenes",
            "--out",
        ])
        .arg(&observation)
        .current_dir(&root)
        .output()
        .expect("run with ambient config");
    assert!(
        output.status.success(),
        "ambient config affected request: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn rendering_is_explicit_and_media_copy_needs_its_own_flag() {
    let root = unique_dir("render-boundary");
    let video = make_video(&root);
    let observation = root.join("observation");
    assert!(
        observe(&video, &observation, &["--capabilities", "probe,scenes"])
            .status
            .success()
    );

    let rendered = Command::new(env!("CARGO_BIN_EXE_video-perception"))
        .args(["observation", "render", observation.to_str().unwrap()])
        .output()
        .expect("render Observation");
    assert!(
        rendered.status.success(),
        "render failed: {}",
        String::from_utf8_lossy(&rendered.stderr)
    );
    let render_stderr = String::from_utf8_lossy(&rendered.stderr);
    assert!(
        render_stderr.contains(&format!(
            "viewer: {}",
            observation.join("index.html").display()
        )),
        "render did not print the Viewer path: {render_stderr}"
    );
    assert!(
        render_stderr.contains("media: not copied (privacy default)"),
        "render did not explain the media default: {render_stderr}"
    );
    for artifact in ["index.html", "report.md", "README.md"] {
        assert!(
            observation.join(artifact).is_file(),
            "render missing {artifact}"
        );
    }
    assert!(
        !observation
            .join("assets")
            .join(video.file_name().unwrap())
            .exists()
    );
    let html = std::fs::read_to_string(observation.join("index.html")).unwrap();
    assert!(html.contains("Complete observation"));
    assert!(html.contains("2 succeeded"));
    assert!(html.contains("Source media was not copied"));

    let with_media = Command::new(env!("CARGO_BIN_EXE_video-perception"))
        .args([
            "observation",
            "render",
            observation.to_str().unwrap(),
            "--include-media",
        ])
        .output()
        .expect("render with media");
    assert!(with_media.status.success());
    assert!(
        String::from_utf8_lossy(&with_media.stderr).contains("media: copied to"),
        "render with media did not print the copied asset path: {}",
        String::from_utf8_lossy(&with_media.stderr)
    );
    assert!(
        observation
            .join("assets")
            .join(video.file_name().unwrap())
            .is_file()
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn observation_query_preserves_progressive_retrieval() {
    let bin = env!("CARGO_BIN_EXE_video-perception");
    let observation = fixtures_dir().join("obs");
    let overview = Command::new(bin)
        .args(["observation", "query", observation.to_str().unwrap()])
        .output()
        .expect("query overview");
    assert!(overview.status.success());
    let dashboard: serde_json::Value = serde_json::from_slice(&overview.stdout).unwrap();
    assert!(dashboard.get("video_identity").is_some());
    assert!(dashboard.get("anomaly_index").is_none());

    let window = Command::new(bin)
        .args([
            "observation",
            "query",
            observation.to_str().unwrap(),
            "--window",
            "scene:1",
            "--track",
            "signalstats",
        ])
        .output()
        .expect("query scene window");
    assert!(
        window.status.success(),
        "query failed: {}",
        String::from_utf8_lossy(&window.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&window.stdout).unwrap();
    assert_eq!(result["signalstats"].as_array().unwrap().len(), 10);

    let frame = Command::new(bin)
        .args([
            "observation",
            "query",
            observation.to_str().unwrap(),
            "--frame",
            "12",
            "--track",
            "signalstats",
        ])
        .output()
        .expect("query frame");
    assert!(frame.status.success());
    assert!(
        serde_json::from_slice::<serde_json::Value>(&frame.stdout).unwrap()["signalstats"]
            .is_object()
    );

    let at = Command::new(bin)
        .args([
            "observation",
            "query",
            observation.to_str().unwrap(),
            "--at",
            "12.5s",
        ])
        .output()
        .expect("query timestamp");
    assert!(at.status.success());
    let at_value: serde_json::Value = serde_json::from_slice(&at.stdout).unwrap();
    assert_eq!(at_value["scene"]["idx"], 1);

    let text = Command::new(bin)
        .args([
            "observation",
            "query",
            observation.to_str().unwrap(),
            "--text",
            "consumer owns",
        ])
        .output()
        .expect("query transcript");
    assert!(text.status.success());
    assert!(String::from_utf8_lossy(&text.stdout).contains("final interpretation"));
}

#[cfg(unix)]
#[test]
fn local_only_plan_never_invokes_network_client() {
    use std::os::unix::fs::PermissionsExt;

    let root = unique_dir("local-only");
    let video = make_video(&root);
    let guarded_bin = root.join("guarded-bin");
    std::fs::create_dir_all(&guarded_bin).unwrap();
    let marker = root.join("curl-invoked");
    let curl = guarded_bin.join("curl");
    std::fs::write(
        &curl,
        "#!/bin/sh\ntouch \"$VIDEO_PERCEPTION_CURL_MARKER\"\nexit 97\n",
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&curl).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&curl, permissions).unwrap();

    let plan = root.join("plan.json");
    std::fs::write(
        &plan,
        r#"{
          "contract": "video-perception.sense-plan/v1",
          "capabilities": [{
            "id": "transcript",
            "required": false,
            "provider": "auto",
            "policy": "local-only"
          }]
        }"#,
    )
    .unwrap();
    let inherited_path = std::env::var("PATH").unwrap_or_default();
    let test_path = format!("{}:{inherited_path}", guarded_bin.display());
    let output = Command::new(env!("CARGO_BIN_EXE_video-perception"))
        .args([
            "observe",
            video.to_str().unwrap(),
            "--plan",
            plan.to_str().unwrap(),
            "--out",
        ])
        .arg(root.join("observation"))
        .env("PATH", test_path)
        .env("VIDEO_PERCEPTION_CURL_MARKER", &marker)
        .env_remove("WHISPER_CPP_BIN")
        .env_remove("WHISPER_CPP_MODEL")
        .output()
        .expect("run local-only plan");
    assert!(
        output.status.success(),
        "local-only optional plan should return evidence: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!marker.exists(), "local-only policy invoked curl");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn invalid_observation_contract_is_rejected() {
    let root = unique_dir("invalid-contract");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("manifest.json"),
        r#"{"contract":"consumer.workspace/v1"}"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_video-perception"))
        .args(["observation", "query", root.to_str().unwrap()])
        .output()
        .expect("query invalid Observation");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported Observation contract"));
    let _ = std::fs::remove_dir_all(root);
}
