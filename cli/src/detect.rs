//! Declarative, read-only detection of what this machine can do.
//!
//! A provider manifest's `detect` block lists checks. Evaluating them never
//! downloads, installs, or runs provider code: a command check looks a name up
//! on PATH, a file check tests paths and simple globs, and a Python check asks
//! `importlib.util.find_spec`, which locates a package without importing it.

use crate::capabilities::Probe;
use crate::extensions::ProviderManifest;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

const BUILTIN_CATALOG: &str = include_str!("builtin_providers.yaml");

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Detect {
    /// Operating systems this provider can run on (`macos`, `linux`, `windows`); empty means any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub platforms: Vec<String>,
    #[serde(default)]
    pub checks: Vec<Check>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing_status: Option<String>,
}

/// One read-only check. Exactly one of `command`, a file check (`env` and/or
/// `file_globs` without `command`), or `python_module` is set. With `command`,
/// `env` names a variable whose value overrides the command path.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    pub id: String,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_globs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub python_module: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub id: String,
    pub met: bool,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Evaluation {
    pub supported_platform: bool,
    pub available: bool,
    pub checks: Vec<CheckResult>,
}

impl Evaluation {
    /// The resolved value of one check, such as an executable or model path.
    pub fn resolved(&self, check_id: &str) -> Option<&str> {
        self.checks
            .iter()
            .find(|check| check.id == check_id && check.met)
            .and_then(|check| check.resolved.as_deref())
    }

    pub fn status(&self, detect: &Detect) -> String {
        if !self.supported_platform {
            "unsupported-on-this-os".to_string()
        } else if self.available {
            detect
                .ready_status
                .clone()
                .unwrap_or_else(|| "ready".to_string())
        } else {
            detect
                .missing_status
                .clone()
                .unwrap_or_else(|| "needs-setup".to_string())
        }
    }
}

pub fn evaluate(detect: &Detect, probe: &dyn Probe) -> Evaluation {
    let os = probe.os();
    let supported_platform = detect.platforms.is_empty() || detect.platforms.contains(&os);
    let checks: Vec<CheckResult> = detect
        .checks
        .iter()
        .map(|check| {
            let resolved = if supported_platform {
                resolve(check, probe)
            } else {
                None
            };
            CheckResult {
                id: check.id.clone(),
                met: resolved.is_some(),
                detail: check.detail.clone(),
                action: resolved.is_none().then(|| check.action.clone()).flatten(),
                resolved,
            }
        })
        .collect();
    Evaluation {
        supported_platform,
        available: supported_platform && checks.iter().all(|check| check.met),
        checks,
    }
}

fn resolve(check: &Check, probe: &dyn Probe) -> Option<String> {
    let from_env = check.env.as_deref().and_then(|name| probe.env(name));
    if let Some(command) = &check.command {
        return match from_env {
            Some(path) => probe.command_path(&path),
            None => probe.command_path(command),
        };
    }
    if let Some(module) = &check.python_module {
        return probe.python_module(module).then(|| module.clone());
    }
    if let Some(path) = from_env {
        return probe.file_exists(&path).then_some(path);
    }
    check
        .file_globs
        .iter()
        .find_map(|pattern| probe.glob(pattern).into_iter().next())
}

pub fn validate(detect: &Detect) -> Result<(), String> {
    let mut seen = HashSet::new();
    for platform in &detect.platforms {
        if !matches!(platform.as_str(), "macos" | "linux" | "windows") {
            return Err(format!(
                "detect platform must be macos, linux, or windows: {platform}"
            ));
        }
    }
    for check in &detect.checks {
        if check.id.trim().is_empty() || check.detail.trim().is_empty() {
            return Err("every detect check needs an id and a detail".to_string());
        }
        if !seen.insert(&check.id) {
            return Err(format!(
                "detect check {} is declared more than once",
                check.id
            ));
        }
        let is_file =
            check.command.is_none() && (check.env.is_some() || !check.file_globs.is_empty());
        let kinds = [
            check.command.is_some(),
            check.python_module.is_some(),
            is_file,
        ]
        .iter()
        .filter(|kind| **kind)
        .count();
        if kinds != 1 || (check.python_module.is_some() && check.env.is_some()) {
            return Err(format!(
                "detect check {} must be exactly one of command, file (env/file_globs), or python_module",
                check.id
            ));
        }
        if let Some(name) = &check.env {
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(format!("detect check {} has an invalid env name", check.id));
            }
        }
        // Top-level names only: find_spec on a dotted name imports the parent package.
        if let Some(module) = &check.python_module {
            if module.is_empty()
                || !module
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                return Err(format!(
                    "detect check {} python_module must be a top-level package name",
                    check.id
                ));
            }
        }
    }
    Ok(())
}

/// The built-in catalog shipped with this binary.
pub fn builtin_catalog() -> Vec<ProviderManifest> {
    serde_yaml::from_str(BUILTIN_CATALOG).expect("built-in provider catalog is valid YAML")
}

/// Evaluate one built-in provider, such as `whisper-cpp`, on the real machine.
pub fn evaluate_builtin(id: &str) -> Option<Evaluation> {
    builtin_catalog()
        .into_iter()
        .find(|provider| provider.id == id)
        .and_then(|provider| provider.detect)
        .map(|detect| evaluate(&detect, &crate::capabilities::SystemProbe))
}

/// Index entry for one provider: what it is, whether it is usable here, and how to call it.
#[derive(Debug, Clone, Serialize)]
pub struct IndexEntry {
    pub id: String,
    pub source: String,
    pub description: String,
    pub runtime: String,
    pub capabilities: Vec<String>,
    pub locality: String,
    pub status: String,
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    pub checks: Vec<CheckResult>,
}

pub fn index_entry(provider: &ProviderManifest, source: &str, probe: &dyn Probe) -> IndexEntry {
    let detect = provider.detect.clone().unwrap_or_default();
    let evaluation = evaluate(&detect, probe);
    IndexEntry {
        id: provider.id.clone(),
        source: source.to_string(),
        description: provider.description.clone(),
        runtime: provider.runtime.kind.clone(),
        capabilities: provider.capabilities.iter().map(|c| c.id.clone()).collect(),
        locality: provider.locality.clone(),
        status: evaluation.status(&detect),
        available: evaluation.available,
        usage: provider.usage.clone(),
        checks: evaluation.checks,
    }
}

/// Expand `~` and match `*` in the last path component. Returns sorted matches.
pub fn glob_paths(pattern: &str, home: Option<&std::path::Path>) -> Vec<String> {
    let expanded = match (pattern.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => home.join(rest),
        (Some(_), None) => return Vec::new(),
        (None, _) => std::path::PathBuf::from(pattern),
    };
    let (Some(dir), Some(name)) = (
        expanded.parent(),
        expanded.file_name().and_then(|n| n.to_str()),
    ) else {
        return Vec::new();
    };
    if !name.contains('*') {
        return if expanded.is_file() {
            vec![expanded.to_string_lossy().into_owned()]
        } else {
            Vec::new()
        };
    }
    let parts: Vec<&str> = name.split('*').collect();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut matches: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_file())
        .filter_map(|entry| {
            let file = entry.file_name().to_string_lossy().into_owned();
            wildcard_match(&parts, &file).then(|| entry.path().to_string_lossy().into_owned())
        })
        .collect();
    matches.sort();
    matches
}

fn wildcard_match(parts: &[&str], name: &str) -> bool {
    let Some((first, rest)) = parts.split_first() else {
        return name.is_empty();
    };
    let Some(mut remaining) = name.strip_prefix(first) else {
        return false;
    };
    let Some((last, middle)) = rest.split_last() else {
        return remaining.is_empty();
    };
    for part in middle {
        match remaining.find(part) {
            Some(at) => remaining = &remaining[at + part.len()..],
            None => return false,
        }
    }
    remaining.len() >= last.len() && remaining.ends_with(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_catalog_parses_and_every_detect_block_validates() {
        let catalog = builtin_catalog();
        assert!(catalog.iter().any(|provider| provider.id == "whisper-cpp"));
        for provider in &catalog {
            validate(
                provider
                    .detect
                    .as_ref()
                    .expect("every built-in declares detect"),
            )
            .unwrap();
            if provider.runtime.kind == "external" {
                assert!(provider.usage.is_some(), "{} needs usage", provider.id);
            }
        }
    }

    #[test]
    fn wildcard_matches_prefix_suffix_and_middle() {
        assert!(wildcard_match(&["ggml-", ".bin"], "ggml-base.bin"));
        assert!(!wildcard_match(
            &["ggml-", ".bin"],
            "for-tests-ggml-tiny.bin"
        ));
        assert!(!wildcard_match(&["ggml-", ".bin"], "ggml-base.gguf"));
        assert!(wildcard_match(&["a", "b", "c"], "a-x-b-y-c"));
    }

    #[test]
    fn a_check_must_be_exactly_one_kind() {
        let bad = Detect {
            checks: vec![Check {
                id: "both".to_string(),
                detail: "x".to_string(),
                action: None,
                command: Some("x".to_string()),
                env: None,
                file_globs: Vec::new(),
                python_module: Some("x".to_string()),
            }],
            ..Detect::default()
        };
        assert!(validate(&bad).is_err());
    }
}
