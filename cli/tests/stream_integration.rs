use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "video-perception-stream-{label}-{}-{nanos}",
        std::process::id()
    ))
}

fn make_video(root: &Path) -> PathBuf {
    make_video_with_duration(root, "1")
}

fn make_video_with_duration(root: &Path, duration_seconds: &str) -> PathBuf {
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
            duration_seconds,
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

#[test]
fn stream_is_a_visible_top_level_execution_mode() {
    let output = Command::new(env!("CARGO_BIN_EXE_video-perception"))
        .arg("--help")
        .output()
        .expect("run --help");
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(
        help.lines()
            .any(|line| line.trim_start().starts_with("stream")),
        "stream command missing from help:\n{help}"
    );
}

#[test]
fn native_stream_sensors_emit_jsonl_and_finalize_a_session() {
    let root = unique_dir("native");
    let video = make_video(&root);
    let session = root.join("session");
    let output = Command::new(env!("CARGO_BIN_EXE_video-perception"))
        .arg("stream")
        .arg(&video)
        .args([
            "--capabilities",
            "keyframes,signalstats",
            "--speed",
            "0",
            "--sample-fps",
            "2",
            "--out",
        ])
        .arg(&session)
        .output()
        .expect("run stream");
    assert!(
        output.status.success(),
        "stream should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let events = String::from_utf8(output.stdout).unwrap();
    let parsed = events
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert!(
        parsed.len() >= 7,
        "expected session, sensor, and close events"
    );
    for (index, event) in parsed.iter().enumerate() {
        assert_eq!(event["contract"], "video-perception.stream-event/v1");
        assert_eq!(event["seq"], (index + 1) as u64);
    }
    assert_eq!(parsed.first().unwrap()["kind"], "session.started");
    assert_eq!(parsed.last().unwrap()["kind"], "session.closed");
    assert_eq!(parsed.last().unwrap()["payload"]["outcome"], "succeeded");
    assert!(
        parsed
            .iter()
            .any(|event| event["capability"] == "keyframes")
    );
    assert!(
        parsed
            .iter()
            .any(|event| event["capability"] == "signalstats")
    );

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(session.join("session.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["contract"], "video-perception.stream-session/v1");
    assert_eq!(manifest["state"], "closed");
    assert_eq!(manifest["event_count"], parsed.len() as u64);
    assert_eq!(
        manifest["sense_plan"]["contract"],
        "video-perception.sense-plan/v1"
    );
    assert_eq!(
        manifest["capabilities"][0]["provider"]["id"],
        "local-ffmpeg"
    );
    assert!(session.join("events.jsonl").is_file());
    assert!(session.join("tracks/keyframes.track.jsonl").is_file());
    assert!(session.join("tracks/signalstats.track.jsonl").is_file());
    assert!(session.join("tracks/keyframes/frame_000001.png").is_file());

    let persisted_events = std::fs::read_to_string(session.join("events.jsonl")).unwrap();
    assert_eq!(persisted_events, events);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn catalog_explains_streaming_strategy_and_runtime_readiness() {
    let output = Command::new(env!("CARGO_BIN_EXE_video-perception"))
        .args(["catalog", "list", "--json"])
        .output()
        .expect("run catalog list");
    assert!(output.status.success());
    let entries: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let entries = entries.as_array().unwrap();
    let keyframes = entries
        .iter()
        .find(|item| item["id"] == "keyframes")
        .unwrap();
    assert_eq!(keyframes["streaming"]["strategy"], "native");
    assert_eq!(keyframes["streaming"]["implemented"], true);
    let transcript = entries
        .iter()
        .find(|item| item["id"] == "transcript")
        .unwrap();
    assert_eq!(transcript["streaming"]["strategy"], "windowed");
    assert_eq!(transcript["streaming"]["implemented"], false);
    assert_eq!(
        transcript["streaming"]["providers"],
        serde_json::json!(["apple-speech", "whisper-cpp"])
    );
}

#[test]
fn stream_rejects_an_adapter_that_is_not_implemented() {
    let root = unique_dir("unsupported");
    let video = make_video(&root);
    let output = Command::new(env!("CARGO_BIN_EXE_video-perception"))
        .arg("stream")
        .arg(&video)
        .args(["--capabilities", "transcript", "--out"])
        .arg(root.join("session"))
        .output()
        .expect("run unsupported stream capability");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("transcript is not available in stream mode"));
    assert!(stderr.contains("strategy: windowed"));
    assert!(!root.join("session").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn finite_stream_can_finalize_the_existing_observation_contract() {
    let root = unique_dir("finalize-observation");
    let video = make_video(&root);
    let session = root.join("session");
    let output = Command::new(env!("CARGO_BIN_EXE_video-perception"))
        .arg("stream")
        .arg(&video)
        .args([
            "--capabilities",
            "keyframes,signalstats",
            "--speed",
            "0",
            "--finalize-observation",
            "--out",
        ])
        .arg(&session)
        .output()
        .expect("run stream with observation finalization");
    assert!(
        output.status.success(),
        "stream finalization should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stream_manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(session.join("session.json")).unwrap())
            .unwrap();
    assert_eq!(stream_manifest["files"]["observation"], "observation");
    let observation_manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(session.join("observation/manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        observation_manifest["contract"],
        "video-perception.observation/v1"
    );
    assert_eq!(
        observation_manifest["sense_plan"],
        stream_manifest["sense_plan"]
    );
    let events = std::fs::read_to_string(session.join("events.jsonl")).unwrap();
    assert!(events.contains("\"kind\":\"session.finalizing\""));
    assert!(events.contains("\"observation\":\"observation\""));
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn interrupt_finalizes_a_cancelled_session() {
    use std::process::Stdio;
    use std::thread;
    use std::time::Duration;

    let root = unique_dir("cancel");
    let video = make_video_with_duration(&root, "5");
    let session = root.join("session");
    let child = Command::new(env!("CARGO_BIN_EXE_video-perception"))
        .arg("stream")
        .arg(&video)
        .args(["--capabilities", "signalstats", "--speed", "1", "--out"])
        .arg(&session)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start stream for cancellation");
    for _ in 0..200 {
        if session.join("session.json").is_file() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        session.join("session.json").is_file(),
        "stream did not publish its running manifest"
    );
    thread::sleep(Duration::from_millis(100));
    // SAFETY: the child id is the live CLI process we just spawned.
    let sent = unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGINT) };
    assert_eq!(sent, 0, "send SIGINT to stream process");
    let output = child.wait_with_output().expect("wait for cancelled stream");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("stream cancelled"),
        "cancelled stream stderr: {stderr}"
    );

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(session.join("session.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["state"], "cancelled");
    assert_eq!(manifest["capabilities"][0]["status"], "failed");
    let events = std::fs::read_to_string(session.join("events.jsonl")).unwrap();
    assert!(events.contains("\"kind\":\"capability.failed\""));
    assert!(events.contains("\"outcome\":\"cancelled\""));
    let _ = std::fs::remove_dir_all(root);
}
