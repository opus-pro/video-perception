//! `video-perception mcp`: the public CLI surface as a local MCP server.
//!
//! The server speaks MCP on stdio. Every tool runs this same executable as a
//! child process and returns what the CLI prints, so stdout contracts, provider
//! routing, and provenance stay in one place. An Observation or Sample
//! directory is the handle a tool returns and a later tool accepts; the CLI can
//! read the same directory. Decision 0004 records the choice.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::service::RequestContext;
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, ServiceExt, schemars, tool, tool_handler,
    tool_router,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::process::Command;

const INSTRUCTIONS: &str = "Video Perception gives an agent local-first video evidence. \
Measure first: call `capabilities` once, then `observe` with deterministic capabilities \
(probe, scenes, stats, signalstats, track, loudnorm) before any model route. `observe` and \
`sample` return a directory path; pass it to `query`, `observation_diff`, or as `sample`'s \
parent. Every tool runs on this machine and sends no media anywhere unless a SensePlan or \
strategy names a remote provider. Timestamps come from decoded evidence; verify any \
model-stated time against them before citing it.";

const STDERR_TAIL_CHARS: usize = 2000;
const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ObserveParams {
    /// Path of a local video file. A relative path resolves against the server's working
    /// directory, which is the folder the client started it in (usually the project folder).
    pub video: String,
    /// Capability ids such as ["probe","scenes","stats"]; the resolver adds technical dependencies. Give exactly one of capabilities, strategy, or plan.
    #[serde(default)]
    pub capabilities: Option<Vec<String>>,
    /// Path or discovered id of a video-perception.strategy/v1 manifest
    #[serde(default)]
    pub strategy: Option<String>,
    /// Path of a video-perception.sense-plan/v1 JSON file
    #[serde(default)]
    pub plan: Option<String>,
    /// Output Observation directory; must not exist yet. Default is a new directory under the evidence root.
    #[serde(default)]
    pub out: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct QueryParams {
    /// Observation directory returned by observe
    pub observation: String,
    /// Time window: scene:3, 10s-20s, or 10000-20000
    #[serde(default)]
    pub window: Option<String>,
    /// Single frame lookup by frame number
    #[serde(default)]
    pub frame: Option<u64>,
    /// Cross-layer lookup at a timestamp such as 12.5s
    #[serde(default)]
    pub at: Option<String>,
    /// Track name such as signalstats, transcript, or color
    #[serde(default)]
    pub track: Option<String>,
    /// Track field such as signalstats.y
    #[serde(default)]
    pub field: Option<String>,
    /// Numeric lower bound, inclusive
    #[serde(default)]
    pub gte: Option<f64>,
    /// Numeric upper bound, inclusive
    #[serde(default)]
    pub lte: Option<f64>,
    /// Transcript substring
    #[serde(default)]
    pub text: Option<String>,
    /// Output format: json (default), text, stats, or spans
    #[serde(default)]
    pub fmt: Option<String>,
    /// Maximum results (default 50)
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SampleParams {
    /// Path of a local video file; remote URLs and playlists are refused. A relative path
    /// resolves against the server's working directory (usually the project folder).
    pub video: String,
    /// New evidence directory; default is a new directory under the evidence root. Existing directories are never overwritten.
    #[serde(default)]
    pub out: Option<String>,
    /// Start in seconds, relative to the first decoded video frame (default 0)
    #[serde(default)]
    pub start: Option<f64>,
    /// Exclusive end in seconds on the same timeline; required to bound work
    pub end: f64,
    /// Maximum sampling rate in frames per second (default 0.2)
    #[serde(default)]
    pub fps: Option<f64>,
    /// Maximum frames this round (default 48); fails before decoding if exceeded
    #[serde(default)]
    pub max_frames: Option<usize>,
    /// Maximum output image edge in pixels (default 960); never upscales
    #[serde(default)]
    pub max_size: Option<u32>,
    /// Integer pixel crop x,y,width,height after display rotation, before scaling
    #[serde(default)]
    pub crop: Option<String>,
    /// Previous round's sample.json; links evidence budgets, not model reasoning
    #[serde(default)]
    pub parent: Option<String>,
    /// Cumulative selected-frame budget across the parent chain (default 240)
    #[serde(default)]
    pub total_frame_budget: Option<usize>,
    /// Wall-clock budget in seconds including the optional sheet (default 60)
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    /// Also build a timestamp-labelled contact sheet with installed ImageMagick (default false)
    #[serde(default)]
    pub sheet: Option<bool>,
    /// Attach images to the result: the sheet when one was built, otherwise individual frames. Default follows `sheet`.
    #[serde(default)]
    pub return_images: Option<bool>,
    /// Maximum individual frames attached when no sheet was built (default 8)
    #[serde(default)]
    pub max_images: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DiffParams {
    /// First Observation directory
    pub a: String,
    /// Second Observation directory
    pub b: String,
}

struct CliOutput {
    success: bool,
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

#[derive(Clone)]
pub struct VideoPerception {
    exe: PathBuf,
    root: PathBuf,
    counter: Arc<AtomicU64>,
    tool_router: ToolRouter<VideoPerception>,
}

#[tool_router]
impl VideoPerception {
    pub fn new() -> Result<Self, String> {
        let exe = std::env::current_exe().map_err(|error| format!("locate executable: {error}"))?;
        Ok(Self {
            exe,
            root: evidence_root(),
            counter: Arc::new(AtomicU64::new(0)),
            tool_router: Self::tool_router(),
        })
    }

    #[tool(
        description = "Report what this machine can run: built-in capabilities, native and user-managed providers, device fit, and routing. Static and read-only; call once per session.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn capabilities(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        self.simple(&["capabilities", "--json"], "capabilities", &ctx)
            .await
    }

    #[tool(
        description = "Verify installed executables and live local services right now. Read-only; it never installs or repairs anything.",
        annotations(read_only_hint = true)
    )]
    async fn doctor(&self, ctx: RequestContext<RoleServer>) -> Result<CallToolResult, McpError> {
        self.simple(&["doctor", "--json"], "doctor", &ctx).await
    }

    #[tool(
        description = "List the logical capability catalog with what each capability detects, its backend, and streaming support.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn catalog_list(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        self.simple(&["catalog", "list", "--json"], "catalog list", &ctx)
            .await
    }

    #[tool(
        description = "List discovered provider, strategy, and knowledge-pack manifests without executing any provider code.",
        annotations(read_only_hint = true)
    )]
    async fn extension_list(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        self.simple(&["extension", "list", "--json"], "extension list", &ctx)
            .await
    }

    #[tool(
        description = "Run a set of capabilities on a local video and write a video-perception.observation/v1 directory. Returns the directory path (the handle for query) and its manifest. Optional capabilities that fail are reported in the manifest, not hidden. A given out must not exist yet; existing directories are never written to.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn observe(
        &self,
        Parameters(params): Parameters<ObserveParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let out = self
            .resolve_out(params.out.as_deref(), "observation")
            .await?;
        let args =
            observe_args(&params, &out).map_err(|error| McpError::invalid_params(error, None))?;
        self.prepare_root().await?;
        let output = self.run_cli(&args, &ctx).await?;
        if !output.success {
            return Ok(cli_failure("observe", &output));
        }
        let manifest_path = out.join("manifest.json");
        let manifest = read_json(&manifest_path).await.map_err(|error| {
            McpError::internal_error(format!("observe finished but {error}"), None)
        })?;
        Ok(CallToolResult::structured(json!({
            "observation": out.to_string_lossy(),
            "manifest": manifest,
            "log": tail(&output.stderr, STDERR_TAIL_CHARS),
        })))
    }

    #[tool(
        description = "Read evidence from an Observation: the overview, a time window, one frame, a cross-layer lookup at a timestamp, or a transcript search. Never runs capabilities.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn query(
        &self,
        Parameters(params): Parameters<QueryParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = query_args(&params);
        let output = self.run_cli(&args, &ctx).await?;
        if !output.success {
            return Ok(cli_failure("query", &output));
        }
        Ok(json_or_text(&output.stdout))
    }

    #[tool(
        description = "Extract a bounded set of timestamped frames from a local video for a closer look: an overview at low fps, then a dense window around an uncertainty, linked by parent. Returns the sample directory, its manifest, and the sheet or frames as images.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn sample(
        &self,
        Parameters(params): Parameters<SampleParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let out = self.resolve_out(params.out.as_deref(), "sample").await?;
        let args = sample_args(&params, &out);
        self.prepare_root().await?;
        let output = self.run_cli(&args, &ctx).await?;
        if !output.success {
            return Ok(cli_failure("sample", &output));
        }
        let manifest: Value = serde_json::from_str(output.stdout.trim()).map_err(|error| {
            McpError::internal_error(
                format!("sample finished but its manifest did not parse: {error}"),
                None,
            )
        })?;
        let want_images = params
            .return_images
            .unwrap_or_else(|| params.sheet.unwrap_or(false));
        let mut notes = Vec::new();
        let mut images = Vec::new();
        if want_images {
            for path in image_paths(&manifest, &out, params.max_images.unwrap_or(8)) {
                match read_image(&path).await {
                    Ok(Some(block)) => images.push(block),
                    Ok(None) => notes.push(format!(
                        "skipped {} (over {} bytes)",
                        path.display(),
                        MAX_IMAGE_BYTES
                    )),
                    Err(error) => notes.push(format!("skipped {}: {error}", path.display())),
                }
            }
        }
        let mut result = CallToolResult::structured(json!({
            "sample": out.to_string_lossy(),
            "manifest": manifest,
            "images_attached": images.len(),
            "notes": notes,
        }));
        result.content.extend(images);
        Ok(result)
    }

    #[tool(
        description = "Compare two Observations and report raw deltas without a verdict.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn observation_diff(
        &self,
        Parameters(params): Parameters<DiffParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args = vec![
            "observation".to_string(),
            "diff".to_string(),
            params.a,
            params.b,
        ];
        let output = self.run_cli(&args, &ctx).await?;
        if !output.success {
            return Ok(cli_failure("observation diff", &output));
        }
        let mut text = output.stdout.trim_end().to_string();
        if !output.stderr.trim().is_empty() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(output.stderr.trim_end());
        }
        Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
    }

    async fn simple(
        &self,
        args: &[&str],
        what: &str,
        ctx: &RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        let output = self.run_cli(&args, ctx).await?;
        if !output.success {
            return Ok(cli_failure(what, &output));
        }
        Ok(json_or_text(&output.stdout))
    }

    async fn run_cli(
        &self,
        args: &[String],
        ctx: &RequestContext<RoleServer>,
    ) -> Result<CliOutput, McpError> {
        let mut command = Command::new(&self.exe);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let child = command.spawn().map_err(|error| {
            McpError::internal_error(format!("spawn {}: {error}", self.exe.display()), None)
        })?;
        let output = tokio::select! {
            output = child.wait_with_output() => output.map_err(|error| {
                McpError::internal_error(format!("wait for {}: {error}", self.exe.display()), None)
            })?,
            _ = ctx.ct.cancelled() => {
                return Err(McpError::internal_error("request cancelled; child process killed", None));
            }
        };
        Ok(CliOutput {
            success: output.status.success(),
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    /// A caller-supplied `out` must be a directory that does not exist yet, so
    /// a tool can never write into or over files the caller already has. The
    /// default is a fresh directory under the evidence root.
    async fn resolve_out(&self, requested: Option<&str>, kind: &str) -> Result<PathBuf, McpError> {
        match requested {
            Some(path) if !path.trim().is_empty() => {
                let path = PathBuf::from(path);
                let exists = tokio::fs::try_exists(&path).await.unwrap_or(true);
                if exists {
                    return Err(McpError::invalid_params(
                        format!(
                            "out already exists: {}; give a new directory or omit out",
                            path.display()
                        ),
                        None,
                    ));
                }
                Ok(path)
            }
            _ => {
                let millis = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|elapsed| elapsed.as_millis())
                    .unwrap_or(0);
                let sequence = self.counter.fetch_add(1, Ordering::Relaxed);
                Ok(self.root.join(format!("{kind}-{millis}-{sequence}")))
            }
        }
    }

    async fn prepare_root(&self) -> Result<(), McpError> {
        tokio::fs::create_dir_all(&self.root)
            .await
            .map_err(|error| {
                McpError::internal_error(
                    format!("create evidence root {}: {error}", self.root.display()),
                    None,
                )
            })
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for VideoPerception {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::from_build_env())
            .with_instructions(INSTRUCTIONS)
    }
}

/// Serve MCP on stdin/stdout until the client disconnects. Nothing else may
/// write to stdout while this runs; diagnostics go to stderr.
pub fn serve() -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("start runtime: {error}"))?;
    runtime.block_on(async {
        let server = VideoPerception::new()?;
        let service = server
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|error| format!("mcp serve: {error}"))?;
        service
            .waiting()
            .await
            .map_err(|error| format!("mcp session: {error}"))?;
        Ok(())
    })
}

fn evidence_root() -> PathBuf {
    std::env::var_os("VIDEO_PERCEPTION_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::data_local_dir().map(|path| path.join("video-perception")))
        .unwrap_or_else(|| PathBuf::from(".video-perception"))
        .join("evidence")
}

fn observe_args(params: &ObserveParams, out: &Path) -> Result<Vec<String>, String> {
    let mut args = vec![
        "observe".to_string(),
        params.video.clone(),
        "--out".to_string(),
        out.to_string_lossy().into_owned(),
    ];
    let selectors = [
        params
            .capabilities
            .as_ref()
            .map(|ids| !ids.is_empty())
            .unwrap_or(false),
        params.strategy.is_some(),
        params.plan.is_some(),
    ]
    .iter()
    .filter(|selected| **selected)
    .count();
    if selectors != 1 {
        return Err("give exactly one of capabilities, strategy, or plan".into());
    }
    if let Some(ids) = &params.capabilities {
        args.push("--capabilities".to_string());
        args.push(ids.join(","));
    }
    if let Some(strategy) = &params.strategy {
        args.push("--strategy".to_string());
        args.push(strategy.clone());
    }
    if let Some(plan) = &params.plan {
        args.push("--plan".to_string());
        args.push(plan.clone());
    }
    Ok(args)
}

fn query_args(params: &QueryParams) -> Vec<String> {
    let mut args = vec![
        "observation".to_string(),
        "query".to_string(),
        params.observation.clone(),
    ];
    let mut flag = |name: &str, value: Option<String>| {
        if let Some(value) = value {
            args.push(format!("--{name}"));
            args.push(value);
        }
    };
    flag("window", params.window.clone());
    flag("frame", params.frame.map(|frame| frame.to_string()));
    flag("at", params.at.clone());
    flag("track", params.track.clone());
    flag("field", params.field.clone());
    flag("gte", params.gte.map(|value| value.to_string()));
    flag("lte", params.lte.map(|value| value.to_string()));
    flag("text", params.text.clone());
    flag("fmt", params.fmt.clone());
    flag("limit", params.limit.map(|limit| limit.to_string()));
    args
}

fn sample_args(params: &SampleParams, out: &Path) -> Vec<String> {
    let mut args = vec![
        "sample".to_string(),
        params.video.clone(),
        "--out".to_string(),
        out.to_string_lossy().into_owned(),
        "--end".to_string(),
        params.end.to_string(),
    ];
    let mut flag = |name: &str, value: Option<String>| {
        if let Some(value) = value {
            args.push(format!("--{name}"));
            args.push(value);
        }
    };
    flag("start", params.start.map(|value| value.to_string()));
    flag("fps", params.fps.map(|value| value.to_string()));
    flag(
        "max-frames",
        params.max_frames.map(|value| value.to_string()),
    );
    flag("max-size", params.max_size.map(|value| value.to_string()));
    flag("crop", params.crop.clone());
    flag("parent", params.parent.clone());
    flag(
        "total-frame-budget",
        params.total_frame_budget.map(|value| value.to_string()),
    );
    flag(
        "timeout-seconds",
        params.timeout_seconds.map(|value| value.to_string()),
    );
    if params.sheet.unwrap_or(false) {
        args.push("--sheet".to_string());
    }
    args
}

fn image_paths(manifest: &Value, out: &Path, max_frames: usize) -> Vec<PathBuf> {
    let resolve = |path: &str| {
        let path = Path::new(path);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            out.join(path)
        }
    };
    if let Some(sheet) = manifest.get("sheet").and_then(Value::as_str) {
        return vec![resolve(sheet)];
    }
    manifest
        .get("frames")
        .and_then(Value::as_array)
        .map(|frames| {
            frames
                .iter()
                .filter_map(|frame| frame.get("path").and_then(Value::as_str))
                .take(max_frames)
                .map(resolve)
                .collect()
        })
        .unwrap_or_default()
}

async fn read_image(path: &Path) -> Result<Option<ContentBlock>, String> {
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|error| format!("stat: {error}"))?;
    if metadata.len() > MAX_IMAGE_BYTES {
        return Ok(None);
    }
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|error| format!("read: {error}"))?;
    let mime = match path.extension().and_then(|ext| ext.to_str()) {
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        _ => "image/jpeg",
    };
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(Some(ContentBlock::image(data, mime)))
}

async fn read_json(path: &Path) -> Result<Value, String> {
    let body = tokio::fs::read_to_string(path)
        .await
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    serde_json::from_str(&body).map_err(|error| format!("parse {}: {error}", path.display()))
}

fn json_or_text(stdout: &str) -> CallToolResult {
    match serde_json::from_str::<Value>(stdout.trim()) {
        Ok(value) => CallToolResult::structured(value),
        Err(_) => CallToolResult::success(vec![ContentBlock::text(stdout.trim_end().to_string())]),
    }
}

fn cli_failure(what: &str, output: &CliOutput) -> CallToolResult {
    let exit = output
        .code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "signal".to_string());
    let mut text = format!("{what} failed (exit {exit})");
    let stderr = tail(&output.stderr, STDERR_TAIL_CHARS);
    if !stderr.is_empty() {
        text.push('\n');
        text.push_str(&stderr);
    }
    CallToolResult::error(vec![ContentBlock::text(text)])
}

fn tail(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    let count = trimmed.chars().count();
    if count <= max_chars {
        return trimmed.to_string();
    }
    trimmed.chars().skip(count - max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observe_params(
        capabilities: Option<Vec<&str>>,
        strategy: Option<&str>,
        plan: Option<&str>,
    ) -> ObserveParams {
        ObserveParams {
            video: "clip.mp4".into(),
            capabilities: capabilities.map(|ids| ids.into_iter().map(String::from).collect()),
            strategy: strategy.map(String::from),
            plan: plan.map(String::from),
            out: None,
        }
    }

    #[test]
    fn observe_args_render_capabilities() {
        let params = observe_params(Some(vec!["probe", "scenes"]), None, None);
        let args = observe_args(&params, Path::new("/tmp/obs")).unwrap();
        assert_eq!(
            args,
            vec![
                "observe",
                "clip.mp4",
                "--out",
                "/tmp/obs",
                "--capabilities",
                "probe,scenes"
            ]
        );
    }

    #[test]
    fn observe_args_require_exactly_one_selector() {
        assert!(observe_args(&observe_params(None, None, None), Path::new("o")).is_err());
        assert!(observe_args(&observe_params(Some(vec![]), None, None), Path::new("o")).is_err());
        assert!(
            observe_args(
                &observe_params(Some(vec!["probe"]), Some("foundation.quick-scan"), None),
                Path::new("o")
            )
            .is_err()
        );
        let strategy = observe_args(
            &observe_params(None, Some("foundation.quick-scan"), None),
            Path::new("o"),
        )
        .unwrap();
        assert_eq!(&strategy[4..], ["--strategy", "foundation.quick-scan"]);
    }

    #[test]
    fn query_args_render_only_given_filters() {
        let params = QueryParams {
            observation: "obs".into(),
            window: Some("scene:3".into()),
            frame: None,
            at: None,
            track: Some("signalstats".into()),
            field: None,
            gte: None,
            lte: Some(80.0),
            text: None,
            fmt: Some("spans".into()),
            limit: Some(10),
        };
        assert_eq!(
            query_args(&params),
            vec![
                "observation",
                "query",
                "obs",
                "--window",
                "scene:3",
                "--track",
                "signalstats",
                "--lte",
                "80",
                "--fmt",
                "spans",
                "--limit",
                "10"
            ]
        );
    }

    #[test]
    fn sample_args_render_bounds_and_sheet() {
        let params = SampleParams {
            video: "clip.mp4".into(),
            out: None,
            start: Some(12.0),
            end: 14.0,
            fps: Some(8.0),
            max_frames: Some(24),
            max_size: None,
            crop: None,
            parent: Some("prev/sample.json".into()),
            total_frame_budget: None,
            timeout_seconds: None,
            sheet: Some(true),
            return_images: None,
            max_images: None,
        };
        assert_eq!(
            sample_args(&params, Path::new("/tmp/s")),
            vec![
                "sample",
                "clip.mp4",
                "--out",
                "/tmp/s",
                "--end",
                "14",
                "--start",
                "12",
                "--fps",
                "8",
                "--max-frames",
                "24",
                "--parent",
                "prev/sample.json",
                "--sheet"
            ]
        );
    }

    #[test]
    fn image_paths_prefer_sheet_then_cap_frames() {
        let out = Path::new("/evidence/s1");
        let with_sheet = json!({"sheet": "sheet.jpg", "frames": [{"path": "frame_0001.jpg"}]});
        assert_eq!(
            image_paths(&with_sheet, out, 8),
            vec![PathBuf::from("/evidence/s1/sheet.jpg")]
        );
        let frames = json!({"sheet": null, "frames": [
            {"path": "frame_0001.jpg"}, {"path": "/abs/frame_0002.jpg"}, {"path": "frame_0003.jpg"}
        ]});
        assert_eq!(
            image_paths(&frames, out, 2),
            vec![
                PathBuf::from("/evidence/s1/frame_0001.jpg"),
                PathBuf::from("/abs/frame_0002.jpg")
            ]
        );
    }

    #[test]
    fn tool_list_is_stable() {
        let mut names: Vec<String> = VideoPerception::tool_router()
            .list_all()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "capabilities",
                "catalog_list",
                "doctor",
                "extension_list",
                "observation_diff",
                "observe",
                "query",
                "sample"
            ]
        );
    }

    #[test]
    fn tail_keeps_last_chars() {
        assert_eq!(tail("  abcdef  ", 3), "def");
        assert_eq!(tail("ab", 3), "ab");
    }

    #[tokio::test]
    async fn resolve_out_refuses_existing_paths_and_mints_fresh_ones() {
        let server = VideoPerception::new().unwrap();
        let existing = std::env::temp_dir();
        let error = server
            .resolve_out(Some(&existing.to_string_lossy()), "observation")
            .await
            .unwrap_err();
        assert!(
            error.message.contains("already exists"),
            "{}",
            error.message
        );

        let fresh = existing.join(format!("vp-mcp-test-{}", std::process::id()));
        let resolved = server
            .resolve_out(Some(&fresh.to_string_lossy()), "observation")
            .await
            .unwrap();
        assert_eq!(resolved, fresh);

        let minted = server.resolve_out(None, "sample").await.unwrap();
        assert!(minted.starts_with(&server.root));
        assert!(
            minted
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("sample-")
        );
        let again = server.resolve_out(Some("  "), "sample").await.unwrap();
        assert_ne!(minted, again);
    }
}
