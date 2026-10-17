use std::process::Command;

#[test]
fn visible_top_level_commands_match_public_contract() {
    let output = Command::new(env!("CARGO_BIN_EXE_video-perception"))
        .arg("--help")
        .output()
        .expect("run --help");
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);

    for command in [
        "capabilities",
        "setup",
        "doctor",
        "update",
        "catalog",
        "extension",
        "observe",
        "observation",
    ] {
        assert!(
            help.lines()
                .any(|line| line.trim_start().starts_with(command)),
            "missing public command {command}:\n{help}"
        );
    }
    for legacy in [
        "config",
        "init",
        "probe",
        "scenes",
        "keyframes",
        "stats",
        "loudnorm",
        "scene-audio",
        "layout",
        "track",
        "beats",
        "transcript",
        "blur",
        "complexity",
        "framerate",
        "flicker",
        "signalstats",
        "avsync",
        "run",
        "sensors",
        "workspace",
        "grep",
        "diff",
        "__capability",
    ] {
        assert!(
            !help
                .lines()
                .any(|line| line.trim_start().starts_with(legacy)),
            "legacy/internal command {legacy} leaked into public help:\n{help}"
        );
    }
}

#[test]
fn legacy_top_level_commands_are_rejected() {
    let bin = env!("CARGO_BIN_EXE_video-perception");
    for legacy in [
        "config",
        "init",
        "probe",
        "scenes",
        "keyframes",
        "stats",
        "loudnorm",
        "scene-audio",
        "layout",
        "track",
        "beats",
        "transcript",
        "blur",
        "complexity",
        "framerate",
        "flicker",
        "signalstats",
        "avsync",
        "run",
        "sensors",
        "workspace",
        "grep",
        "diff",
    ] {
        let output = Command::new(bin)
            .arg(legacy)
            .output()
            .expect("run legacy command");
        assert!(
            !output.status.success(),
            "legacy command {legacy} must fail"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("unrecognized subcommand"),
            "legacy command {legacy} should fail at the public parser"
        );
    }
}

#[test]
fn observe_requires_exactly_one_request_form() {
    let bin = env!("CARGO_BIN_EXE_video-perception");
    let missing = Command::new(bin)
        .args(["observe", "video.mp4"])
        .output()
        .expect("run observe without request");
    assert!(!missing.status.success());

    let both = Command::new(bin)
        .args([
            "observe",
            "video.mp4",
            "--plan",
            "plan.json",
            "--capabilities",
            "probe",
        ])
        .output()
        .expect("run observe with both request forms");
    assert!(!both.status.success());
}

#[test]
fn doctor_has_no_mutating_fix_path() {
    let output = Command::new(env!("CARGO_BIN_EXE_video-perception"))
        .args(["doctor", "--fix"])
        .output()
        .expect("run doctor --fix");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unexpected argument '--fix'"));
}

#[test]
fn catalog_is_the_single_public_capability_index() {
    let output = Command::new(env!("CARGO_BIN_EXE_video-perception"))
        .args(["catalog", "list", "--json"])
        .output()
        .expect("run catalog list");
    assert!(output.status.success());
    let catalog: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(catalog.as_array().is_some_and(|items| items.len() >= 20));
}
