#[cfg(unix)]
mod unix {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "video-perception-update-test-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    fn write_fake_curl(directory: &Path) -> PathBuf {
        fs::create_dir_all(directory).unwrap();
        let curl = directory.join("curl");
        fs::write(
            &curl,
            r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$VIDEO_PERCEPTION_TEST_CURL_LOG"
destination=""
url=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--output" ]; then
    shift
    destination="$1"
  elif [ "${1#http}" != "$1" ]; then
    url="$1"
  fi
  shift
done
if [ -z "$destination" ]; then
  printf '{"tag_name":"%s"}\n' "$VIDEO_PERCEPTION_TEST_RELEASE_TAG"
else
  case "$url" in
    */checksums.txt) cp "$VIDEO_PERCEPTION_TEST_CHECKSUMS" "$destination" ;;
    *) cp "$VIDEO_PERCEPTION_TEST_ARCHIVE" "$destination" ;;
  esac
fi
exit 0
"#,
        )
        .unwrap();
        fs::set_permissions(&curl, fs::Permissions::from_mode(0o755)).unwrap();
        curl
    }

    fn test_path(fake_bin: &Path) -> String {
        format!(
            "{}:{}",
            fake_bin.display(),
            std::env::var("PATH").unwrap_or_default()
        )
    }

    fn platform_asset(version: &str) -> String {
        let (os, arch) = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            ("darwin", "arm64")
        } else if cfg!(all(
            target_os = "linux",
            target_arch = "x86_64",
            target_env = "musl"
        )) {
            ("linux", "amd64_musl")
        } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            ("linux", "amd64")
        } else {
            panic!("self-update integration test has no release asset for this target")
        };
        format!("video-perception_{version}_{os}_{arch}.tar.gz")
    }

    fn sha256(path: &Path) -> String {
        for (program, args) in [("shasum", vec!["-a", "256"]), ("sha256sum", vec![])] {
            let output = Command::new(program).args(args).arg(path).output();
            if let Ok(output) = output
                && output.status.success()
            {
                return String::from_utf8(output.stdout)
                    .unwrap()
                    .split_whitespace()
                    .next()
                    .unwrap()
                    .to_string();
            }
        }
        panic!("test requires shasum or sha256sum");
    }

    #[test]
    fn update_check_reports_a_newer_release_without_downloading() {
        let root = test_root("check");
        let fake_bin = root.join("bin");
        write_fake_curl(&fake_bin);
        let log = root.join("curl.log");

        let output = Command::new(env!("CARGO_BIN_EXE_video-perception"))
            .args(["update", "--check", "--version", "v9.9.9"])
            .env("PATH", test_path(&fake_bin))
            .env("VIDEO_PERCEPTION_TEST_CURL_LOG", &log)
            .env("VIDEO_PERCEPTION_TEST_RELEASE_TAG", "v9.9.9")
            .output()
            .expect("run update --check");

        assert!(
            output.status.success(),
            "update --check failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("Update available"));
        assert!(stdout.contains("video-perception update --version v9.9.9"));
        assert!(
            !log.exists(),
            "an explicit --version check needs no network call"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn update_downloads_verifies_and_replaces_a_binary_copy() {
        let version = env!("CARGO_PKG_VERSION");
        let tag = format!("v{version}");
        let asset = platform_asset(version);
        let root = test_root("replace");
        let fake_bin = root.join("bin");
        let payload = root.join("payload");
        let installed = root.join("installed");
        fs::create_dir_all(&payload).unwrap();
        fs::create_dir_all(&installed).unwrap();
        write_fake_curl(&fake_bin);

        let source_binary = Path::new(env!("CARGO_BIN_EXE_video-perception"));
        let payload_binary = payload.join("video-perception");
        let installed_binary = installed.join("video-perception");
        fs::copy(source_binary, &payload_binary).unwrap();
        fs::copy(source_binary, &installed_binary).unwrap();
        fs::set_permissions(&payload_binary, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&installed_binary, fs::Permissions::from_mode(0o755)).unwrap();

        let archive = root.join(&asset);
        let tar = Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&payload)
            .arg("video-perception")
            .status()
            .expect("create release archive");
        assert!(tar.success());
        let checksums = root.join("checksums.txt");
        fs::write(&checksums, format!("{}  {asset}\n", sha256(&archive))).unwrap();
        let log = root.join("curl.log");

        let output = Command::new(&installed_binary)
            .args(["update", "--version", &tag, "--force"])
            .env("PATH", test_path(&fake_bin))
            .env("VIDEO_PERCEPTION_TEST_CURL_LOG", &log)
            .env("VIDEO_PERCEPTION_TEST_RELEASE_TAG", &tag)
            .env("VIDEO_PERCEPTION_TEST_ARCHIVE", &archive)
            .env("VIDEO_PERCEPTION_TEST_ASSET", &asset)
            .env("VIDEO_PERCEPTION_TEST_CHECKSUMS", &checksums)
            .output()
            .expect("run full self-update");

        assert!(
            output.status.success(),
            "self-update failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("Updated video-perception"));
        assert!(
            fs::read_to_string(&log)
                .unwrap()
                .contains("releases/download")
        );

        let version_output = Command::new(&installed_binary)
            .arg("--version")
            .output()
            .expect("run replaced binary");
        assert!(version_output.status.success());
        assert!(String::from_utf8_lossy(&version_output.stdout).contains(version));
        let _ = fs::remove_dir_all(root);
    }
}
