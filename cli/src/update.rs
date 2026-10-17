use semver::Version;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;
use std::process::{Command, Output};

const RELEASE_REPOSITORY: &str = "opus-pro/video-perception";
const BINARY_NAME: &str = "video-perception";

pub struct UpdateOptions<'a> {
    pub check: bool,
    pub requested_version: Option<&'a str>,
    pub force: bool,
}

pub fn execute(options: UpdateOptions<'_>) -> Result<(), String> {
    let current = Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|error| format!("invalid current version: {error}"))?;
    let requested_tag = options
        .requested_version
        .map(normalize_release_tag)
        .transpose()?;
    let target_tag = resolve_release_tag(requested_tag.as_deref())?;
    let target = parse_release_version(&target_tag)?;

    match target.cmp(&current) {
        Ordering::Equal if options.check => {
            println!("video-perception {current} is up to date ({target_tag}).");
            return Ok(());
        }
        Ordering::Equal if !options.force => {
            println!("video-perception {current} is already installed. Use --force to reinstall.");
            return Ok(());
        }
        Ordering::Less if options.check => {
            println!(
                "Installed video-perception {current} is newer than the selected release {target_tag}."
            );
            return Ok(());
        }
        Ordering::Less if options.requested_version.is_none() => {
            println!(
                "Installed video-perception {current} is newer than the latest release {target_tag}; nothing changed."
            );
            return Ok(());
        }
        Ordering::Less if !options.force => {
            return Err(format!(
                "refusing to downgrade video-perception {current} to {target_tag}; pass --force to confirm"
            ));
        }
        Ordering::Greater if options.check => {
            println!("Update available: video-perception {current} -> {target} ({target_tag}).");
            if options.requested_version.is_some() {
                println!("Run `video-perception update --version {target_tag}` to install it.");
            } else {
                println!("Run `video-perception update` to install it.");
            }
            return Ok(());
        }
        Ordering::Equal | Ordering::Less | Ordering::Greater => {}
    }

    let platform = Platform::detect()?;
    let asset_name = format!(
        "{BINARY_NAME}_{}_{}_{}.tar.gz",
        target_tag.trim_start_matches('v'),
        platform.os,
        platform.arch
    );
    let temp = crate::temp::PrivateDir::create("update")?;

    eprintln!(
        "Downloading {target_tag} for {}/{}...",
        platform.os, platform.arch
    );
    download_release(&target_tag, &asset_name, temp.path())?;

    let archive_path = temp.path().join(&asset_name);
    let checksums_path = temp.path().join("checksums.txt");
    verify_release_checksum(&archive_path, &checksums_path, &asset_name)?;
    eprintln!("Checksum verified: {asset_name}");

    extract_binary(&archive_path, temp.path())?;
    let candidate = temp.path().join(BINARY_NAME);
    validate_candidate(&candidate, &target)?;

    let current_executable = std::env::current_exe().map_err(|error| {
        format!("cannot locate the running video-perception executable: {error}")
    })?;
    replace_executable(&candidate, &current_executable)?;
    validate_candidate(&current_executable, &target)?;

    println!(
        "Updated video-perception {current} -> {target} at {}",
        current_executable.display()
    );
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct Platform {
    os: &'static str,
    arch: &'static str,
}

impl Platform {
    fn detect() -> Result<Self, String> {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => Ok(Self {
                os: "darwin",
                arch: "arm64",
            }),
            ("linux", "x86_64") if cfg!(target_env = "musl") => Ok(Self {
                os: "linux",
                arch: "amd64_musl",
            }),
            ("linux", "x86_64") => Ok(Self {
                os: "linux",
                arch: "amd64",
            }),
            (os, arch) => Err(format!(
                "self-update has no published release asset for `{os}/{arch}`"
            )),
        }
    }
}

fn normalize_release_tag(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("--version cannot be empty".into());
    }
    let unprefixed = value.strip_prefix('v').unwrap_or(value);
    Version::parse(unprefixed)
        .map_err(|error| format!("invalid release version `{value}`: {error}"))?;
    Ok(format!("v{unprefixed}"))
}

fn parse_release_version(tag: &str) -> Result<Version, String> {
    Version::parse(tag.strip_prefix('v').unwrap_or(tag))
        .map_err(|error| format!("release returned invalid tag `{tag}`: {error}"))
}

fn resolve_release_tag(requested: Option<&str>) -> Result<String, String> {
    if let Some(tag) = requested {
        return normalize_release_tag(tag);
    }
    let url = format!("https://api.github.com/repos/{RELEASE_REPOSITORY}/releases/latest");
    let output = Command::new("curl")
        .args(["--fail", "--silent", "--show-error", "--location"])
        .arg(&url)
        .output()
        .map_err(|error| format!("failed to query GitHub Releases: {error}"))?;
    let body = successful_stdout(output, "query GitHub Releases")?;
    let release: serde_json::Value = serde_json::from_str(&body)
        .map_err(|error| format!("GitHub Releases returned invalid JSON: {error}"))?;
    let tag = release
        .get("tag_name")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "GitHub Releases response has no tag_name".to_string())?;
    normalize_release_tag(tag)
}

fn download_release(tag: &str, asset_name: &str, directory: &Path) -> Result<(), String> {
    let base = format!("https://github.com/{RELEASE_REPOSITORY}/releases/download/{tag}");
    for filename in [asset_name, "checksums.txt"] {
        let destination = directory.join(filename);
        let output = Command::new("curl")
            .args([
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--retry",
                "3",
            ])
            .arg("--output")
            .arg(&destination)
            .arg(format!("{base}/{filename}"))
            .output()
            .map_err(|error| {
                format!("failed to download {filename} from release {tag}: {error}")
            })?;
        successful_stdout(output, &format!("download {filename} from release {tag}"))?;
    }

    for path in [directory.join(asset_name), directory.join("checksums.txt")] {
        if !path.is_file() {
            return Err(format!(
                "release {tag} did not contain expected asset `{}`",
                path.file_name().unwrap_or_default().to_string_lossy()
            ));
        }
    }
    Ok(())
}

fn successful_stdout(output: Output, action: &str) -> Result<String, String> {
    if output.status.success() {
        return String::from_utf8(output.stdout)
            .map_err(|error| format!("{action} returned non-UTF-8 output: {error}"));
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!("failed to {action}: {}", stderr.trim()))
}

fn verify_release_checksum(
    archive_path: &Path,
    checksums_path: &Path,
    asset_name: &str,
) -> Result<(), String> {
    let checksums = fs::read_to_string(checksums_path)
        .map_err(|error| format!("read {}: {error}", checksums_path.display()))?;
    let expected = checksums
        .lines()
        .find_map(|line| {
            let mut fields = line.split_whitespace();
            let digest = fields.next()?;
            let filename = fields.next()?.trim_start_matches('*');
            (filename == asset_name).then_some(digest)
        })
        .ok_or_else(|| format!("checksums.txt has no checksum for `{asset_name}`"))?;
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "checksums.txt contains an invalid SHA-256 for `{asset_name}`"
        ));
    }

    let mut file = File::open(archive_path)
        .map_err(|error| format!("open {}: {error}", archive_path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("read {}: {error}", archive_path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(format!(
            "checksum mismatch for `{asset_name}`: expected {expected}, got {actual}"
        ));
    }
    Ok(())
}

fn extract_binary(archive_path: &Path, directory: &Path) -> Result<(), String> {
    let output = Command::new("tar")
        .arg("-xzf")
        .arg(archive_path)
        .arg("-C")
        .arg(directory)
        .arg(BINARY_NAME)
        .output()
        .map_err(|error| format!("failed to run tar: {error}"))?;
    successful_stdout(output, "extract the release binary")?;

    let candidate = directory.join(BINARY_NAME);
    if !candidate.is_file() {
        return Err(format!(
            "release archive did not contain a root `{BINARY_NAME}` executable"
        ));
    }
    Ok(())
}

fn validate_candidate(path: &Path, expected: &Version) -> Result<(), String> {
    let output = Command::new(path)
        .arg("--version")
        .output()
        .map_err(|error| format!("cannot run downloaded binary {}: {error}", path.display()))?;
    let version_output = successful_stdout(output, "validate the downloaded binary")?;
    let actual = version_output
        .split_whitespace()
        .last()
        .ok_or_else(|| "downloaded binary returned an empty version".to_string())?;
    let actual = Version::parse(actual).map_err(|error| {
        format!("downloaded binary returned invalid version `{actual}`: {error}")
    })?;
    if &actual != expected {
        return Err(format!(
            "downloaded binary reports version {actual}, expected {expected}"
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn replace_executable(candidate: &Path, current: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let parent = current
        .parent()
        .ok_or_else(|| format!("executable has no parent directory: {}", current.display()))?;
    let staged = parent.join(format!(".{BINARY_NAME}.update-{}", std::process::id()));
    let direct_result = (|| -> io::Result<()> {
        fs::copy(candidate, &staged)?;
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o755))?;
        File::open(&staged)?.sync_all()?;
        fs::rename(&staged, current)?;
        Ok(())
    })();

    match direct_result {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            let _ = fs::remove_file(&staged);
            replace_with_sudo(candidate, current, &staged)
        }
        Err(error) => {
            let _ = fs::remove_file(&staged);
            Err(format!("replace {}: {error}", current.display()))
        }
    }
}

#[cfg(unix)]
fn replace_with_sudo(candidate: &Path, current: &Path, staged: &Path) -> Result<(), String> {
    eprintln!(
        "{} is not writable; administrator permission is required.",
        current.display()
    );
    let install = Command::new("sudo")
        .args(["install", "-m", "0755"])
        .arg(candidate)
        .arg(staged)
        .status()
        .map_err(|error| format!("failed to run sudo install: {error}"))?;
    if !install.success() {
        return Err(format!(
            "administrator install failed; rerun the original installer for {}",
            current.display()
        ));
    }
    let replace = Command::new("sudo")
        .args(["mv", "-f"])
        .arg(staged)
        .arg(current)
        .status()
        .map_err(|error| format!("failed to run sudo mv: {error}"))?;
    if !replace.success() {
        return Err(format!(
            "administrator replacement failed; verified binary remains at {}",
            staged.display()
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn replace_executable(_candidate: &Path, _current: &Path) -> Result<(), String> {
    Err("self-update is supported only on macOS and Linux".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_tags_accept_prefixed_and_plain_semver() {
        assert_eq!(normalize_release_tag("0.5.1").unwrap(), "v0.5.1");
        assert_eq!(normalize_release_tag("v0.5.1-rc.1").unwrap(), "v0.5.1-rc.1");
    }

    #[test]
    fn release_tags_reject_paths_and_non_semver_values() {
        assert!(normalize_release_tag("../../latest").is_err());
        assert!(normalize_release_tag("main").is_err());
        assert!(normalize_release_tag("").is_err());
    }

    #[test]
    fn current_release_platform_is_supported() {
        let platform = Platform::detect().unwrap();
        if cfg!(target_os = "macos") {
            assert_eq!(
                platform,
                Platform {
                    os: "darwin",
                    arch: "arm64"
                }
            );
        } else if cfg!(target_env = "musl") {
            assert_eq!(
                platform,
                Platform {
                    os: "linux",
                    arch: "amd64_musl",
                }
            );
        } else {
            assert_eq!(
                platform,
                Platform {
                    os: "linux",
                    arch: "amd64"
                }
            );
        }
    }
}
