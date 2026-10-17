mod capabilities;
mod commands;
mod compare;
mod doctor;
mod extensions;
mod mcp;
mod plan;
mod provider;
mod registry;
mod run;
mod sample;
mod sensors;
mod stream;
mod temp;
mod update;
mod util;
mod workspace;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "video-perception",
    version,
    about = "Consumer-agnostic media observation and streaming runtime",
    long_about = "Turn an explicit SensePlan into a reusable Observation or an incremental event stream.\n\n\
        Consumers choose capabilities and interpret the evidence. Video Perception resolves\n\
        technical dependencies and providers without owning product profiles or rules.",
    after_help = "Quick start:\n\
        \x20 video-perception capabilities --json\n\
        \x20 video-perception doctor --json\n\
        \x20 video-perception observe video.mp4 --capabilities probe,scenes,stats --out observation/\n\
        \x20 video-perception stream video.mp4 --capabilities keyframes,signalstats --out session/\n\
        \x20 video-perception observation query observation/"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Gather bounded, timestamped visual evidence for an agent's next look (local files only)
    Sample(sample::SampleOptions),
    /// Inspect this machine's built-in and user-managed capability paths
    #[command(
        long_about = "Describe what this machine can run now and what each upgrade path requires.\n\n\
            This command is read-only: it performs no downloads, configuration writes,\n\
            remote probes, or media processing. Use `doctor` to verify live services."
    )]
    Capabilities {
        /// Output the stable video-perception.capabilities/v1 contract
        #[arg(long)]
        json: bool,
    },

    /// Generate a read-only provider and privacy setup plan
    Setup {
        /// Profile: basic, mac-native, or local-model
        #[arg(long)]
        profile: Option<String>,
        /// Output the stable video-perception.setup-plan/v1 contract
        #[arg(long)]
        json: bool,
    },

    /// Verify installed tools and live provider services without changing them
    Doctor {
        /// Output machine-readable JSON
        #[arg(long)]
        json: bool,
    },

    /// Check for or install a verified Video Perception release
    #[command(
        long_about = "Check GitHub Releases for a newer Video Perception binary or replace this executable with a verified release.\n\n\
            Downloaded assets are matched to this OS/architecture, verified against checksums.txt,\n\
            and version-checked before the current executable is replaced."
    )]
    Update {
        /// Report release availability without changing the executable
        #[arg(long, conflicts_with = "force")]
        check: bool,
        /// Install or inspect a specific release, for example v0.5.1
        #[arg(long)]
        version: Option<String>,
        /// Reinstall the same version or explicitly allow a downgrade
        #[arg(long)]
        force: bool,
    },

    /// Inspect the logical capability catalog
    #[command(subcommand)]
    Catalog(CatalogCommand),

    /// Discover and validate decentralized providers, strategies, and knowledge packs
    #[command(subcommand)]
    Extension(ExtensionCommand),

    /// Execute an explicit SensePlan and produce a video-perception.observation/v1 artifact
    #[command(
        long_about = "Execute exactly the requested capabilities plus their technical dependencies.\n\n\
            Use a SensePlan JSON file, a strategy manifest/id, or a comma-separated capability list.\n\
            The default artifact contains reusable machine data only: no source-video copy,\n\
            generated report, README, or HTML viewer."
    )]
    Observe {
        /// Source media asset
        video: String,
        /// Output Observation directory
        #[arg(long, default_value = "observation")]
        out: String,
        /// Path to a video-perception.sense-plan/v1 JSON file
        #[arg(
            long,
            conflicts_with_all = ["capabilities", "strategy"],
            required_unless_present_any = ["capabilities", "strategy"]
        )]
        plan: Option<PathBuf>,
        /// Path or discovered id of a video-perception.strategy/v1 manifest
        #[arg(
            long,
            conflicts_with_all = ["plan", "capabilities"],
            required_unless_present_any = ["plan", "capabilities"]
        )]
        strategy: Option<String>,
        /// Convenience capability list using registry required/optional defaults
        #[arg(
            long,
            value_delimiter = ',',
            conflicts_with_all = ["plan", "strategy"],
            required_unless_present_any = ["plan", "strategy"]
        )]
        capabilities: Option<Vec<String>>,
    },

    /// Continuously emit video-perception.stream-event/v1 JSONL while media is arriving
    #[command(
        long_about = "Run stream-capable sensors incrementally and write each event to stdout as JSONL.\n\n\
            A durable session manifest and event log are written under --out. File inputs\n\
            replay at realtime speed by default; use --speed 0 for unpaced evaluation.\n\
            Run `catalog list --json` to inspect per-capability streaming readiness."
    )]
    Stream {
        /// FFmpeg-readable file or network source
        source: String,
        /// Output stream-session directory
        #[arg(long, default_value = "stream-session")]
        out: String,
        /// Path to an video-perception.sense-plan/v1 JSON file
        #[arg(
            long,
            conflicts_with = "capabilities",
            required_unless_present = "capabilities"
        )]
        plan: Option<PathBuf>,
        /// Convenience capability list using registry required/optional defaults
        #[arg(
            long,
            value_delimiter = ',',
            conflicts_with = "plan",
            required_unless_present = "plan"
        )]
        capabilities: Option<Vec<String>>,
        /// File replay speed; 1 is realtime and 0 is unpaced
        #[arg(long, default_value = "1")]
        speed: f64,
        /// Sampling rate used by native visual stream sensors
        #[arg(long, default_value = "2")]
        sample_fps: f64,
        /// Maximum pending events before backpressure is applied
        #[arg(long, default_value = "256")]
        event_buffer: usize,
        /// Backpressure policy for measurement events
        #[arg(long, value_enum, default_value = "block")]
        backpressure: stream::BackpressurePolicy,
        /// Override a capability provider, for example keyframes=local-ffmpeg
        #[arg(long = "provider", value_name = "CAPABILITY=PROVIDER")]
        providers: Vec<String>,
        /// After end-of-stream, also produce a batch video-perception.observation/v1 artifact
        #[arg(long)]
        finalize_observation: bool,
    },

    /// Query, compare, or render an existing Observation
    #[command(subcommand)]
    Observation(ObservationCommand),

    /// Serve the public CLI surface as a local MCP server over stdio
    Mcp,

    /// Internal capability process protocol; not a public product interface
    #[command(name = "__capability", hide = true, subcommand)]
    InternalCapability(InternalCapabilityCommand),
}

#[derive(Subcommand)]
enum CatalogCommand {
    /// List every registered logical capability
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Search capability descriptions
    Search {
        query: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum ExtensionCommand {
    /// Show the explicit, user, and project extension search paths
    Paths {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// List all discovered extension manifests without executing provider code
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Validate one manifest or every manifest below a directory
    Validate {
        path: PathBuf,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum ObservationCommand {
    /// Retrieve aggregate, windowed, frame, timestamp, or transcript evidence
    Query {
        /// Observation directory
        #[arg(default_value = "observation")]
        observation: String,
        /// Time window: scene:3, 10s-20s, or 10000-20000
        #[arg(long)]
        window: Option<String>,
        /// Single frame lookup by frame number
        #[arg(long)]
        frame: Option<u64>,
        /// Cross-layer query at a timestamp such as 12.5s
        #[arg(long)]
        at: Option<String>,
        /// Track name such as signalstats or color
        #[arg(long)]
        track: Option<String>,
        /// Track field such as signalstats.y
        #[arg(long)]
        field: Option<String>,
        /// Numeric lower bound, inclusive
        #[arg(long)]
        gte: Option<f64>,
        /// Numeric upper bound, inclusive
        #[arg(long)]
        lte: Option<f64>,
        /// Transcript substring
        #[arg(long)]
        text: Option<String>,
        /// Output format: json, text, stats, or spans
        #[arg(long, default_value = "json")]
        fmt: String,
        /// Maximum results
        #[arg(long, default_value = "50")]
        limit: usize,
    },
    /// Compare two Observations and report raw deltas without a verdict
    Diff { a: String, b: String },
    /// Generate human-readable report and HTML artifacts after execution
    Render {
        observation: String,
        /// Explicitly copy the source asset into the Observation
        #[arg(long)]
        include_media: bool,
    },
}

#[derive(Subcommand)]
enum InternalCapabilityCommand {
    Probe {
        video: String,
    },
    Scenes {
        video: String,
        #[arg(long, default_value = "0.3")]
        threshold: f64,
        #[arg(long, default_value = "800")]
        merge: u64,
        #[arg(long)]
        no_merge: bool,
    },
    Keyframes {
        video: String,
        #[arg(long, default_value = "signals")]
        out_dir: String,
    },
    Stats {
        video: String,
        #[arg(long)]
        scenes: String,
        #[arg(long, default_value = "stats")]
        out_dir: String,
    },
    Loudnorm {
        video: String,
    },
    SceneAudio {
        video: String,
        #[arg(long)]
        scenes: String,
    },
    Layout {
        video: String,
        #[arg(long)]
        scenes: String,
        #[arg(long, default_value = "layout")]
        out_dir: String,
        #[arg(long)]
        keyframe_dir: Option<String>,
    },
    Track {
        video: String,
        #[arg(long)]
        scenes: String,
        #[arg(long, default_value = "track")]
        out_dir: String,
    },
    Beats {
        video: String,
    },
    Transcript {
        video: String,
        #[arg(long)]
        lang: Option<String>,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        policy: Option<String>,
    },
    Blur {
        video: String,
        #[arg(long)]
        scenes: String,
        #[arg(long, default_value = "stats")]
        keyframe_dir: String,
    },
    Complexity {
        video: String,
        #[arg(long)]
        scenes: String,
        #[arg(long, default_value = "stats")]
        keyframe_dir: String,
    },
    Framerate {
        video: String,
    },
    Flicker {
        video: String,
    },
    Signalstats {
        video: String,
        #[arg(long)]
        scenes: Option<String>,
        #[arg(long)]
        track_dir: Option<String>,
    },
    Avsync {
        video: String,
    },
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Commands::Sample(options) => sample::execute(options),
        Commands::Capabilities { json } => capabilities::execute(json),
        Commands::Setup { profile, json } => capabilities::setup(profile.as_deref(), json),
        Commands::Doctor { json } => doctor::execute(json),
        Commands::Update {
            check,
            version,
            force,
        } => update::execute(update::UpdateOptions {
            check,
            requested_version: version.as_deref(),
            force,
        }),
        Commands::Catalog(command) => match command {
            CatalogCommand::List { json } => sensors::list(json),
            CatalogCommand::Search { query, json } => sensors::search(&query, json),
        },
        Commands::Extension(command) => match command {
            ExtensionCommand::Paths { json } => extensions::paths(json),
            ExtensionCommand::List { json } => extensions::list(json),
            ExtensionCommand::Validate { path, json } => extensions::validate(&path, json),
        },
        Commands::Observe {
            video,
            out,
            plan: plan_path,
            strategy,
            capabilities,
        } => {
            let sense_plan = match (plan_path, strategy, capabilities) {
                (Some(path), None, None) => plan::SensePlan::load(&path),
                (None, Some(strategy), None) => {
                    extensions::resolve_strategy(&strategy).map(|strategy| strategy.plan)
                }
                (None, None, Some(ids)) => plan::SensePlan::from_capabilities(&ids),
                _ => Err("provide exactly one of --plan, --strategy, or --capabilities".into()),
            };
            sense_plan.and_then(|plan| workspace::observe(&video, &out, &plan))
        }
        Commands::Stream {
            source,
            out,
            plan: plan_path,
            capabilities,
            speed,
            sample_fps,
            event_buffer,
            backpressure,
            providers,
            finalize_observation,
        } => {
            let sense_plan = match (plan_path, capabilities) {
                (Some(path), None) => plan::SensePlan::load(&path),
                (None, Some(ids)) => plan::SensePlan::from_capabilities(&ids),
                _ => Err("provide exactly one of --plan or --capabilities".into()),
            };
            sense_plan.and_then(|plan| {
                stream::execute(stream::StreamOptions {
                    source: &source,
                    out: &out,
                    plan: &plan,
                    speed,
                    sample_fps,
                    event_buffer,
                    backpressure,
                    provider_overrides: &providers,
                    finalize_observation,
                })
            })
        }
        Commands::Observation(command) => match command {
            ObservationCommand::Query {
                observation,
                window,
                frame,
                at,
                track,
                field,
                gte,
                lte,
                text,
                fmt,
                limit,
            } => workspace::grep::execute(
                &observation,
                window.as_deref(),
                field.as_deref(),
                gte,
                lte,
                text.as_deref(),
                limit,
                frame,
                track.as_deref(),
                at.as_deref(),
                &fmt,
            ),
            ObservationCommand::Diff { a, b } => compare::execute_diff(&a, &b),
            ObservationCommand::Render {
                observation,
                include_media,
            } => workspace::render(&observation, include_media),
        },
        Commands::Mcp => mcp::serve(),
        Commands::InternalCapability(command) => execute_internal_capability(command),
    };

    if let Err(error) = result {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn execute_internal_capability(command: InternalCapabilityCommand) -> Result<(), String> {
    match command {
        InternalCapabilityCommand::Probe { video } => commands::probe::execute(&video),
        InternalCapabilityCommand::Scenes {
            video,
            threshold,
            merge,
            no_merge,
        } => commands::scenes::execute(&video, threshold, merge, !no_merge),
        InternalCapabilityCommand::Keyframes { video, out_dir } => {
            commands::keyframes::execute(&video, &out_dir)
        }
        InternalCapabilityCommand::Stats {
            video,
            scenes,
            out_dir,
        } => commands::stats::execute(&video, &scenes, &out_dir),
        InternalCapabilityCommand::Loudnorm { video } => commands::loudnorm::execute(&video),
        InternalCapabilityCommand::SceneAudio { video, scenes } => {
            commands::scene_audio::execute(&video, &scenes)
        }
        InternalCapabilityCommand::Layout {
            video,
            scenes,
            out_dir,
            keyframe_dir,
        } => commands::layout::execute(&video, &scenes, &out_dir, keyframe_dir.as_deref()),
        InternalCapabilityCommand::Track {
            video,
            scenes,
            out_dir,
        } => commands::track::execute(&video, &scenes, &out_dir),
        InternalCapabilityCommand::Beats { video } => commands::beats::execute(&video),
        InternalCapabilityCommand::Transcript {
            video,
            lang,
            provider,
            policy,
        } => commands::transcript::execute(
            &video,
            lang.as_deref(),
            provider.as_deref(),
            policy.as_deref(),
        ),
        InternalCapabilityCommand::Blur {
            video,
            scenes,
            keyframe_dir,
        } => commands::blur::execute(&video, &scenes, &keyframe_dir),
        InternalCapabilityCommand::Complexity {
            video,
            scenes,
            keyframe_dir,
        } => commands::complexity::execute(&video, &scenes, &keyframe_dir),
        InternalCapabilityCommand::Framerate { video } => commands::framerate::execute(&video),
        InternalCapabilityCommand::Flicker { video } => commands::flicker::execute(&video),
        InternalCapabilityCommand::Signalstats {
            video,
            scenes,
            track_dir,
        } => commands::signalstats::execute(&video, scenes.as_deref(), track_dir.as_deref()),
        InternalCapabilityCommand::Avsync { video } => commands::avsync::execute(&video),
    }
}
