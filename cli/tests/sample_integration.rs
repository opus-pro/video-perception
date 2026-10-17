use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

fn directory() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "perception-sample-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    dir
}

fn fixture(dir: &PathBuf, variable: bool) -> PathBuf {
    let path = dir.join("input.mp4");
    let mut ffmpeg = Command::new("ffmpeg");
    ffmpeg.args([
        "-v",
        "error",
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=320x180:rate=24:duration=3",
    ]);
    if variable {
        ffmpeg.args(["-vf", "select='not(mod(n,5))'", "-fps_mode", "vfr"]);
    }
    assert!(
        ffmpeg
            .args(["-c:v", "mpeg4"])
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    path
}

fn run(source: &PathBuf, out: &PathBuf, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_video-perception"))
        .arg("sample")
        .arg(source)
        .arg("--out")
        .arg(out)
        .args(extra)
        .output()
        .unwrap()
}

#[test]
fn sample_links_rounds_preserves_pts_and_does_not_overwrite() {
    let dir = directory();
    let source = fixture(&dir, true);
    let out = dir.join("overview");
    let output = run(
        &source,
        &out,
        &["--start", "0.1", "--end", "2.1", "--fps", "4", "--sheet"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let first: Value = serde_json::from_slice(&output.stdout).unwrap();
    let frames = first["frames"].as_array().unwrap();
    // First qualifying VFR frame is 5/24 s, not the requested start or index / 24.
    assert!((frames[0]["timestamp_seconds"].as_f64().unwrap() - 5.0 / 24.0).abs() < 0.0001);
    assert!(frames.len() <= 8);
    assert_eq!(first["provenance"]["locality"], "local");
    assert!(first["provenance"]["model"].is_null());
    for frame in frames {
        assert!(out.join(frame["path"].as_str().unwrap()).is_file());
    }
    let snapshot = fs::read(out.join("sample.json")).unwrap();
    assert!(!run(&source, &out, &["--end", "2"]).status.success());
    assert_eq!(fs::read(out.join("sample.json")).unwrap(), snapshot);
    let parent = out.join("sample.json");
    let output = run(
        &source,
        &dir.join("detail"),
        &[
            "--start",
            "1",
            "--end",
            "2",
            "--fps",
            "24",
            "--crop",
            "10,10,100,100",
            "--max-size",
            "64",
            "--parent",
            parent.to_str().unwrap(),
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let second: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(second["round"], 2);
    assert_eq!(
        second["total_frames"].as_u64().unwrap(),
        frames.len() as u64 + second["frames"].as_array().unwrap().len() as u64
    );
    assert!(
        second["frames"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| (1.0..2.0).contains(&f["timestamp_seconds"].as_f64().unwrap()))
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn invalid_requests_and_exhausted_budget_do_not_start_work() {
    let dir = directory();
    let source = fixture(&dir, false);
    for extra in [
        vec!["--end", "NaN"],
        vec!["--end", "2", "--fps", "100"],
        vec!["--end", "2", "--fps", "0"],
        vec!["--end", "2", "--crop", "0,0,0,2"],
        vec!["--end", "2", "--total-frame-budget", "0"],
    ] {
        let out = dir.join("invalid");
        assert!(!run(&source, &out, &extra).status.success());
        assert!(!out.exists());
    }
    let first = dir.join("first");
    assert!(
        run(
            &source,
            &first,
            &["--end", "2", "--fps", "1", "--total-frame-budget", "2"]
        )
        .status
        .success()
    );
    let parent = first.join("sample.json");
    assert!(
        !run(
            &source,
            &dir.join("second"),
            &["--end", "1", "--parent", parent.to_str().unwrap()]
        )
        .status
        .success()
    );
    let empty = dir.join("empty");
    assert!(
        !run(&source, &empty, &["--start", "20", "--end", "21"])
            .status
            .success()
    );
    assert!(!empty.join("sample.json").exists());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_playlist_cannot_fetch_remote_media() {
    let dir = directory();
    let source = dir.join("remote.m3u8");
    fs::write(&source, "#EXTM3U\n#EXT-X-TARGETDURATION:10\n#EXTINF:10,\nhttp://127.0.0.1:9/video.ts\n#EXT-X-ENDLIST\n").unwrap();
    let out = dir.join("blocked");
    assert!(!run(&source, &out, &["--end", "1"]).status.success());
    assert!(!out.join("sample.json").exists());
    let _ = fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn timeout_kills_tool_without_publishing_completed_evidence() {
    use std::os::unix::fs::PermissionsExt;
    let dir = directory();
    let tools = dir.join("tools");
    fs::create_dir(&tools).unwrap();
    let stub = tools.join("ffmpeg");
    fs::write(&stub, "#!/bin/sh\nexec /bin/sleep 10\n").unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
    let input = dir.join("input.mp4");
    fs::write(&input, "stub input").unwrap();
    let out = dir.join("timed-out");
    let started = std::time::Instant::now();
    let result = Command::new(env!("CARGO_BIN_EXE_video-perception"))
        .arg("sample")
        .arg(input)
        .arg("--out")
        .arg(&out)
        .args(["--end", "1", "--timeout-seconds", "1"])
        .env("PATH", tools)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(started.elapsed().as_secs() < 5);
    assert!(!out.join("sample.json").exists());
    let _ = fs::remove_dir_all(dir);
}
