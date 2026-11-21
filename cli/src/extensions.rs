//! Decentralized extension discovery and validation.
//!
//! Extensions are data owned by the user or current project. Discovery is
//! read-only and never executes provider code. A provider is executed only
//! when a SensePlan explicitly selects it.

use crate::plan::SensePlan;
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

pub const PROVIDER_CONTRACT: &str = "video-perception.provider/v1";
pub const STRATEGY_CONTRACT: &str = "video-perception.strategy/v1";
pub const KNOWLEDGE_PACK_CONTRACT: &str = "video-perception.knowledge-pack/v1";
pub const PROVIDER_STDIO_CONTRACT: &str = "video-perception.provider-stdio/v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderManifest {
    pub contract: String,
    pub id: String,
    pub version: String,
    pub description: String,
    pub locality: String,
    pub media_transfer: String,
    pub runtime: ProviderRuntime,
    pub capabilities: Vec<ProviderCapability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Read-only checks that say whether this provider can run on the current machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detect: Option<crate::detect::Detect>,
    /// For `external` runtimes: how the agent invokes the tool itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderRuntime {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_env: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderCapability {
    pub id: String,
    pub contract: String,
    #[serde(default = "default_video_mode")]
    pub modes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrategyManifest {
    pub contract: String,
    pub id: String,
    pub version: String,
    pub description: String,
    pub plan: SensePlan,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guidance: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgePackManifest {
    pub contract: String,
    pub id: String,
    pub version: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,
    pub knowledge: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Manifest {
    Provider(ProviderManifest),
    Strategy(StrategyManifest),
    KnowledgePack(KnowledgePackManifest),
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtensionRoot {
    pub source: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
struct ExtensionRecord {
    kind: &'static str,
    id: String,
    version: String,
    path: PathBuf,
    source: String,
    manifest: Value,
}

#[derive(Debug, Serialize)]
struct DiscoveryReport {
    contract: &'static str,
    roots: Vec<ExtensionRoot>,
    extensions: Vec<ExtensionRecord>,
    errors: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ValidationReport {
    contract: &'static str,
    valid: bool,
    extensions: Vec<ExtensionRecord>,
    errors: Vec<String>,
}

impl Manifest {
    pub fn id(&self) -> &str {
        match self {
            Self::Provider(value) => &value.id,
            Self::Strategy(value) => &value.id,
            Self::KnowledgePack(value) => &value.id,
        }
    }

    fn version(&self) -> &str {
        match self {
            Self::Provider(value) => &value.version,
            Self::Strategy(value) => &value.version,
            Self::KnowledgePack(value) => &value.version,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::Provider(_) => "provider",
            Self::Strategy(_) => "strategy",
            Self::KnowledgePack(_) => "knowledge-pack",
        }
    }

    fn as_json(&self) -> Result<Value, String> {
        match self {
            Self::Provider(value) => serde_json::to_value(value),
            Self::Strategy(value) => serde_json::to_value(value),
            Self::KnowledgePack(value) => serde_json::to_value(value),
        }
        .map_err(|error| format!("encode extension manifest: {error}"))
    }
}

pub fn paths(json_output: bool) -> Result<(), String> {
    let roots = extension_roots()?;
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "contract": "video-perception.extension-paths/v1",
                "roots": roots,
            }))
            .map_err(|error| format!("encode extension paths: {error}"))?
        );
    } else {
        for root in roots {
            eprintln!("{:<12} {}", root.source, root.path.display());
        }
    }
    Ok(())
}

pub fn list(json_output: bool) -> Result<(), String> {
    let report = discover()?;
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|error| format!("encode extension catalog: {error}"))?
        );
    } else {
        for extension in &report.extensions {
            eprintln!(
                "{:<16} {:<36} {:<10} {}",
                extension.kind,
                extension.id,
                extension.version,
                extension.path.display()
            );
        }
        if report.extensions.is_empty() {
            eprintln!("No user or project extensions discovered.");
        }
        for error in &report.errors {
            eprintln!("! {error}");
        }
    }
    Ok(())
}

/// Return extension discovery as structured data without executing providers.
///
/// Discovery failures are represented in the value so callers such as
/// `capabilities` can remain useful even when one extension is malformed.
pub fn discovery_value() -> Value {
    match discover() {
        Ok(report) => serde_json::to_value(report).unwrap_or_else(|error| {
            json!({
                "contract": "video-perception.extensions/v1",
                "roots": [],
                "extensions": [],
                "errors": [format!("encode extension catalog: {error}")],
            })
        }),
        Err(error) => json!({
            "contract": "video-perception.extensions/v1",
            "roots": [],
            "extensions": [],
            "errors": [error],
        }),
    }
}

pub fn validate(path: &Path, json_output: bool) -> Result<(), String> {
    let files = manifest_files(path)?;
    if files.is_empty() {
        return Err(format!(
            "no YAML or JSON extension manifests found at {}",
            path.display()
        ));
    }

    let mut extensions = Vec::new();
    let mut errors = Vec::new();
    for file in files {
        match load_manifest(&file) {
            Ok(manifest) => extensions.push(record(manifest, file, "explicit")?),
            Err(error) => errors.push(error),
        }
    }
    detect_duplicates(&extensions, &mut errors);
    let report = ValidationReport {
        contract: "video-perception.extension-validation/v1",
        valid: errors.is_empty(),
        extensions,
        errors,
    };

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|error| format!("encode extension validation: {error}"))?
        );
    } else if report.valid {
        for extension in &report.extensions {
            eprintln!(
                "\u{2713} {} {} ({})",
                extension.kind, extension.id, extension.version
            );
        }
    } else {
        for error in &report.errors {
            eprintln!("\u{2717} {error}");
        }
    }

    if report.valid {
        Ok(())
    } else {
        Err("extension validation failed".to_string())
    }
}

pub fn resolve_strategy(input: &str) -> Result<StrategyManifest, String> {
    let input_path = Path::new(input);
    if input_path.is_file() {
        return match load_manifest(input_path)? {
            Manifest::Strategy(strategy) => Ok(strategy),
            other => Err(format!(
                "{} contains a {}, not a strategy",
                input_path.display(),
                other.kind()
            )),
        };
    }

    let (manifests, errors) = discovered_manifests()?;
    let mut matches = manifests
        .into_iter()
        .filter_map(|(_, manifest, _)| (manifest.id() == input).then_some(manifest))
        .collect::<Vec<_>>();
    match matches.len() {
        0 => Err(format!(
            "strategy {input} was not found{}; run `video-perception extension list --json`",
            discovery_warning(&errors)
        )),
        1 => match matches.remove(0) {
            Manifest::Strategy(strategy) => Ok(strategy),
            other => Err(format!(
                "extension {input} is a {}, not a strategy",
                other.kind()
            )),
        },
        _ => Err(format!(
            "strategy {input} is ambiguous because multiple manifests use that id"
        )),
    }
}

pub fn find_provider(id: &str, capability: &str) -> Result<Option<ProviderManifest>, String> {
    Ok(find_provider_by_id(id)?.filter(|provider| {
        provider
            .capabilities
            .iter()
            .any(|candidate| candidate.id == capability)
    }))
}

pub fn find_provider_by_id(id: &str) -> Result<Option<ProviderManifest>, String> {
    let (manifests, _) = discovered_manifests()?;
    let mut matches = manifests
        .into_iter()
        .filter_map(|(_, manifest, _)| (manifest.id() == id).then_some(manifest))
        .collect::<Vec<_>>();
    match matches.len() {
        0 => Ok(None),
        1 => match matches.remove(0) {
            Manifest::Provider(provider) => Ok(Some(provider)),
            other => Err(format!(
                "extension {id} is a {}, not a provider",
                other.kind()
            )),
        },
        _ => Err(format!(
            "provider {id} is ambiguous because multiple manifests use that id"
        )),
    }
}

pub fn execute_command_provider(
    provider: &ProviderManifest,
    capability: &str,
    request: Value,
) -> Result<String, String> {
    if provider.runtime.kind != "command" {
        return Err(format!(
            "provider {} uses runtime {}; only command providers implement the stdio execution protocol",
            provider.id, provider.runtime.kind
        ));
    }
    let executable = provider
        .runtime
        .command
        .first()
        .ok_or_else(|| format!("provider {} has no executable", provider.id))?;
    let mut command = Command::new(executable);
    command.args(provider.runtime.command.iter().skip(1));
    command.arg("execute");
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("start provider {}: {error}", provider.id))?;
    let body = serde_json::to_vec(&json!({
        "contract": "video-perception.provider-request/v1",
        "provider": provider.id,
        "capability": capability,
        "mode": "video",
        "request": request,
    }))
    .map_err(|error| format!("encode provider request: {error}"))?;
    child
        .stdin
        .take()
        .ok_or_else(|| format!("provider {} stdin is unavailable", provider.id))?
        .write_all(&body)
        .map_err(|error| format!("write provider {} request: {error}", provider.id))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("wait for provider {}: {error}", provider.id))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "provider {} exited with {}: {}",
            provider.id,
            output.status,
            stderr.trim()
        ));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("provider {} returned invalid JSON: {error}", provider.id))?;
    if !value.is_object() && !value.is_array() {
        return Err(format!(
            "provider {} returned JSON that is not an object or array",
            provider.id
        ));
    }
    serde_json::to_string_pretty(&value)
        .map_err(|error| format!("encode provider {} output: {error}", provider.id))
}

fn discover() -> Result<DiscoveryReport, String> {
    let roots = extension_roots()?;
    let mut extensions = Vec::new();
    let mut errors = Vec::new();
    for root in &roots {
        if !root.path.is_dir() {
            continue;
        }
        for file in manifest_files(&root.path)? {
            match load_manifest(&file) {
                Ok(manifest) => match record(manifest, file, &root.source) {
                    Ok(record) => extensions.push(record),
                    Err(error) => errors.push(error),
                },
                Err(error) => errors.push(error),
            }
        }
    }
    detect_duplicates(&extensions, &mut errors);
    extensions.sort_by(|left, right| {
        left.kind
            .cmp(right.kind)
            .then_with(|| left.id.cmp(&right.id))
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(DiscoveryReport {
        contract: "video-perception.extensions/v1",
        roots,
        extensions,
        errors,
    })
}

/// Every valid provider manifest in the extension roots, with its root's source.
pub fn discovered_providers() -> Vec<(ProviderManifest, String)> {
    discovered_manifests()
        .map(|(found, _)| {
            found
                .into_iter()
                .filter_map(|(_, manifest, source)| match manifest {
                    Manifest::Provider(provider) => Some((provider, source)),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn discovered_manifests() -> Result<(Vec<(PathBuf, Manifest, String)>, Vec<String>), String> {
    let mut found = Vec::new();
    let mut errors = Vec::new();
    for root in extension_roots()? {
        if !root.path.is_dir() {
            continue;
        }
        for file in manifest_files(&root.path)? {
            match load_manifest(&file) {
                Ok(manifest) => found.push((file, manifest, root.source.clone())),
                Err(error) => errors.push(error),
            }
        }
    }
    Ok((found, errors))
}

fn discovery_warning(errors: &[String]) -> String {
    if errors.is_empty() {
        String::new()
    } else {
        format!(" ({} other manifest(s) are invalid)", errors.len())
    }
}

pub fn extension_roots() -> Result<Vec<ExtensionRoot>, String> {
    let cwd =
        std::env::current_dir().map_err(|error| format!("read current directory: {error}"))?;
    let mut roots = Vec::new();
    if let Some(paths) = std::env::var_os("VIDEO_PERCEPTION_EXTENSION_PATH") {
        for path in std::env::split_paths(&paths) {
            roots.push(ExtensionRoot {
                source: "explicit".to_string(),
                path,
            });
        }
    }

    let user_base = std::env::var_os("VIDEO_PERCEPTION_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::config_dir().map(|path| path.join("video-perception")));
    if let Some(user_base) = user_base {
        roots.push(ExtensionRoot {
            source: "user".to_string(),
            path: user_base.join("extensions"),
        });
    }
    roots.push(ExtensionRoot {
        source: "project".to_string(),
        path: cwd.join(".video-perception/extensions"),
    });

    let mut seen = HashSet::new();
    roots.retain(|root| seen.insert(root.path.clone()));
    Ok(roots)
}

fn load_manifest(path: &Path) -> Result<Manifest, String> {
    let body = fs::read_to_string(path)
        .map_err(|error| format!("read extension {}: {error}", path.display()))?;
    let header: BTreeMap<String, Value> = serde_yaml::from_str(&body)
        .map_err(|error| format!("parse extension {}: {error}", path.display()))?;
    let contract = header
        .get("contract")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("extension {} is missing contract", path.display()))?;
    let manifest = match contract {
        PROVIDER_CONTRACT => Manifest::Provider(
            serde_yaml::from_str(&body)
                .map_err(|error| format!("parse provider {}: {error}", path.display()))?,
        ),
        STRATEGY_CONTRACT => Manifest::Strategy(
            serde_yaml::from_str(&body)
                .map_err(|error| format!("parse strategy {}: {error}", path.display()))?,
        ),
        KNOWLEDGE_PACK_CONTRACT => Manifest::KnowledgePack(
            serde_yaml::from_str(&body)
                .map_err(|error| format!("parse knowledge pack {}: {error}", path.display()))?,
        ),
        other => {
            return Err(format!(
                "extension {} uses unsupported contract {other}",
                path.display()
            ));
        }
    };
    validate_manifest(&manifest, path)?;
    Ok(manifest)
}

fn validate_manifest(manifest: &Manifest, path: &Path) -> Result<(), String> {
    validate_id(manifest.id())?;
    Version::parse(manifest.version())
        .map_err(|error| format!("{} has invalid version: {error}", path.display()))?;
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    match manifest {
        Manifest::Provider(provider) => validate_provider(provider, base),
        Manifest::Strategy(strategy) => {
            require_text(&strategy.description, "strategy description")?;
            strategy.plan.validate()
        }
        Manifest::KnowledgePack(pack) => {
            require_text(&pack.description, "knowledge pack description")?;
            if pack.knowledge.is_empty() {
                return Err("knowledge pack must reference at least one knowledge file".to_string());
            }
            for relative in &pack.knowledge {
                validate_relative_file(base, relative, "knowledge")?;
            }
            Ok(())
        }
    }
}

fn validate_provider(provider: &ProviderManifest, base: &Path) -> Result<(), String> {
    require_text(&provider.description, "provider description")?;
    if !matches!(provider.locality.as_str(), "local" | "remote") {
        return Err("provider locality must be local or remote".to_string());
    }
    if !matches!(
        provider.media_transfer.as_str(),
        "never" | "possible" | "required"
    ) {
        return Err("provider media_transfer must be never, possible, or required".to_string());
    }
    if provider.locality == "remote" && provider.media_transfer == "never" {
        return Err("a remote provider cannot declare media_transfer: never".to_string());
    }
    if provider.capabilities.is_empty() {
        return Err("provider must implement at least one capability".to_string());
    }
    match provider.runtime.kind.as_str() {
        "command" => {
            if provider.runtime.command.is_empty() {
                return Err("command provider runtime requires command".to_string());
            }
            if provider.runtime.protocol.as_deref() != Some(PROVIDER_STDIO_CONTRACT) {
                return Err(format!(
                    "command provider runtime must use protocol {PROVIDER_STDIO_CONTRACT}"
                ));
            }
            if provider.runtime.base_url.is_some()
                || provider.runtime.base_url_env.is_some()
                || provider.runtime.credential_env.is_some()
            {
                return Err(
                    "command provider runtime cannot declare URL or credential fields".to_string(),
                );
            }
        }
        "openai-compatible" | "ollama-compatible" => {
            if !provider.runtime.command.is_empty() || provider.runtime.protocol.is_some() {
                return Err(format!(
                    "{} provider runtime cannot declare command or protocol",
                    provider.runtime.kind
                ));
            }
            if provider.runtime.base_url.is_none() && provider.runtime.base_url_env.is_none() {
                return Err(format!(
                    "{} provider runtime requires base_url or base_url_env",
                    provider.runtime.kind
                ));
            }
            if let Some(name) = &provider.runtime.base_url_env {
                validate_env_name(name, "base_url_env")?;
            }
            if let Some(name) = &provider.runtime.credential_env {
                validate_env_name(name, "credential_env")?;
            }
        }
        "native" | "external" => {
            if !provider.runtime.command.is_empty()
                || provider.runtime.protocol.is_some()
                || provider.runtime.base_url.is_some()
                || provider.runtime.base_url_env.is_some()
                || provider.runtime.credential_env.is_some()
            {
                return Err(format!(
                    "{} provider runtime cannot declare adapter fields",
                    provider.runtime.kind
                ));
            }
        }
        other => return Err(format!("unsupported provider runtime type: {other}")),
    }
    if provider.runtime.kind == "external"
        && provider
            .usage
            .as_deref()
            .is_none_or(|u| u.trim().is_empty())
    {
        return Err("an external provider must say how the agent invokes it in usage".to_string());
    }
    if let Some(detect) = &provider.detect {
        crate::detect::validate(detect)?;
    }

    let mut seen = HashSet::new();
    for capability in &provider.capabilities {
        if !seen.insert(&capability.id) {
            return Err(format!(
                "provider {} declares capability {} more than once",
                provider.id, capability.id
            ));
        }
        require_text(&capability.id, "capability id")?;
        require_text(&capability.contract, "capability contract")?;
        if capability.modes.is_empty()
            || capability
                .modes
                .iter()
                .any(|mode| !matches!(mode.as_str(), "video" | "streaming"))
        {
            return Err(format!(
                "provider {} capability {} modes must contain video or streaming",
                provider.id, capability.id
            ));
        }
        if let Some(relative) = &capability.output_schema {
            validate_relative_file(base, relative, "output_schema")?;
        }
    }
    Ok(())
}

fn validate_id(id: &str) -> Result<(), String> {
    if !id.contains('.')
        || id.starts_with('.')
        || id.ends_with('.')
        || !id.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || ".-_".contains(character)
        })
    {
        return Err(format!(
            "extension id {id:?} must be a lowercase namespaced id such as user.my-provider"
        ));
    }
    Ok(())
}

fn validate_env_name(name: &str, label: &str) -> Result<(), String> {
    let mut characters = name.chars();
    let valid_first = characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic());
    if !valid_first
        || !characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
    {
        return Err(format!(
            "provider runtime {label} must be an environment variable name"
        ));
    }
    Ok(())
}

fn validate_relative_file(base: &Path, relative: &str, label: &str) -> Result<(), String> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!(
            "{label} must be a path inside the extension: {relative}"
        ));
    }
    if !base.join(path).is_file() {
        return Err(format!(
            "{label} file does not exist: {}",
            base.join(path).display()
        ));
    }
    Ok(())
}

fn require_text(value: &str, label: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{label} must not be empty"))
    } else {
        Ok(())
    }
}

fn manifest_files(path: &Path) -> Result<Vec<PathBuf>, String> {
    if path.is_file() {
        return Ok(if is_manifest_file(path) {
            vec![path.to_path_buf()]
        } else {
            Vec::new()
        });
    }
    if !path.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    collect_manifest_files(path, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_manifest_files(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(path)
        .map_err(|error| format!("read extension directory {}: {error}", path.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("read extension entry: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("inspect {}: {error}", entry.path().display()))?;
        if file_type.is_dir() {
            collect_manifest_files(&entry.path(), files)?;
        } else if file_type.is_file() && is_manifest_file(&entry.path()) {
            files.push(entry.path());
        }
    }
    Ok(())
}

fn is_manifest_file(path: &Path) -> bool {
    match path.extension().and_then(|value| value.to_str()) {
        Some("yaml" | "yml") => true,
        Some("json") => path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| {
                matches!(
                    name,
                    "provider.json" | "strategy.json" | "pack.json" | "manifest.json"
                ) || name.ends_with(".extension.json")
            }),
        _ => false,
    }
}

fn record(manifest: Manifest, path: PathBuf, source: &str) -> Result<ExtensionRecord, String> {
    Ok(ExtensionRecord {
        kind: manifest.kind(),
        id: manifest.id().to_string(),
        version: manifest.version().to_string(),
        path,
        source: source.to_string(),
        manifest: manifest.as_json()?,
    })
}

fn detect_duplicates(extensions: &[ExtensionRecord], errors: &mut Vec<String>) {
    let mut seen: HashMap<&str, (&str, &Path)> = HashMap::new();
    for extension in extensions {
        if let Some((previous_kind, previous_path)) =
            seen.insert(extension.id.as_str(), (extension.kind, &extension.path))
        {
            errors.push(format!(
                "duplicate extension id {} used by {} at {} and {} at {}",
                extension.id,
                previous_kind,
                previous_path.display(),
                extension.kind,
                extension.path.display()
            ));
        }
    }
}

fn default_video_mode() -> Vec<String> {
    vec!["video".to_string()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("video-perception-{label}-{unique}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn validates_decentralized_provider_manifest() {
        let dir = temp_dir("provider");
        let path = dir.join("provider.yaml");
        fs::write(
            &path,
            r#"contract: video-perception.provider/v1
id: user.example
version: 0.1.0
description: Example local provider
locality: local
media_transfer: never
runtime:
  type: command
  command: [example-provider]
  protocol: video-perception.provider-stdio/v1
capabilities:
  - id: transcript
    contract: video-perception.transcript/v1
    modes: [video]
license: MIT
"#,
        )
        .unwrap();

        let manifest = load_manifest(&path).unwrap();
        assert_eq!(manifest.id(), "user.example");
        assert!(matches!(manifest, Manifest::Provider(_)));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn strategy_reuses_and_validates_the_sense_plan_contract() {
        let dir = temp_dir("strategy");
        let path = dir.join("strategy.yaml");
        fs::write(
            &path,
            r#"contract: video-perception.strategy/v1
id: user.quick-scan
version: 0.1.0
description: Quick deterministic scan
plan:
  contract: video-perception.sense-plan/v1
  capabilities:
    - id: probe
    - id: scenes
guidance:
  - Inspect scene boundaries before escalating to a model.
license: MIT
"#,
        )
        .unwrap();

        let manifest = load_manifest(&path).unwrap();
        assert!(matches!(manifest, Manifest::Strategy(_)));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn knowledge_files_must_stay_inside_the_pack() {
        let dir = temp_dir("knowledge");
        let path = dir.join("pack.yaml");
        fs::write(
            &path,
            r#"contract: video-perception.knowledge-pack/v1
id: user.unsafe
version: 0.1.0
description: Unsafe path
knowledge: [../secret.md]
"#,
        )
        .unwrap();

        let error = load_manifest(&path).unwrap_err();
        assert!(error.contains("inside the extension"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn command_provider_uses_versioned_json_stdio_without_a_shell() {
        let dir = temp_dir("command-provider");
        let script = dir.join("provider.sh");
        fs::write(
            &script,
            r#"#!/bin/sh
cat >/dev/null
printf '{"contract":"video-perception.example/v1","ok":true}'
"#,
        )
        .unwrap();
        let provider = ProviderManifest {
            contract: PROVIDER_CONTRACT.to_string(),
            id: "user.example-command".to_string(),
            version: "0.1.0".to_string(),
            description: "Example command provider".to_string(),
            locality: "local".to_string(),
            media_transfer: "never".to_string(),
            runtime: ProviderRuntime {
                kind: "command".to_string(),
                command: vec!["/bin/sh".to_string(), script.display().to_string()],
                protocol: Some(PROVIDER_STDIO_CONTRACT.to_string()),
                base_url: None,
                base_url_env: None,
                credential_env: None,
            },
            capabilities: vec![ProviderCapability {
                id: "transcript".to_string(),
                contract: "video-perception.transcript/v1".to_string(),
                modes: vec!["video".to_string()],
                model: None,
                output_schema: None,
            }],
            license: Some("MIT".to_string()),
            detect: None,
            usage: None,
            notes: Vec::new(),
        };

        let output = execute_command_provider(
            &provider,
            "transcript",
            json!({"video": "/tmp/example.mp4"}),
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["ok"], true);
        let _ = fs::remove_dir_all(dir);
    }
}
