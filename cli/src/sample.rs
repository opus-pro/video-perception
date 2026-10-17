//! Bounded visual evidence, not an autonomous planner or a model call.
use clap::Args;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, UNIX_EPOCH};

const CONTRACT: &str = "video-perception.sample/v1";

#[derive(Args, Debug)]
pub struct SampleOptions {
    /// Existing local video. Remote URLs and playlists are not accepted.
    pub video: PathBuf,
    /// New evidence directory; existing directories are never overwritten
    #[arg(long)]
    pub out: PathBuf,
    /// Start in seconds, relative to the first decoded video frame
    #[arg(long, default_value_t = 0.0)]
    pub start: f64,
    /// Exclusive end in seconds on the same timeline (required to bound work)
    #[arg(long)]
    pub end: f64,
    /// Maximum sampling rate; selects source frames without duplicating them
    #[arg(long, default_value_t = 0.2)]
    pub fps: f64,
    /// Maximum frames this round; fails before decoding if the request exceeds it
    #[arg(long, default_value_t = 48)]
    pub max_frames: usize,
    /// Maximum output image edge in pixels, preserving aspect ratio; never upscale
    #[arg(long, default_value_t = 960)]
    pub max_size: u32,
    /// Optional integer pixel crop x,y,width,height after display rotation, before scaling
    #[arg(long)]
    pub crop: Option<String>,
    /// Previous round's sample.json; links evidence, not model reasoning
    #[arg(long)]
    pub parent: Option<PathBuf>,
    /// Cumulative selected-frame budget across the parent chain
    #[arg(long, default_value_t = 240)]
    pub total_frame_budget: usize,
    /// Total wall-clock processing budget, including optional montage
    #[arg(long, default_value_t = 60)]
    pub timeout_seconds: u64,
    /// Also create a timestamp-labelled contact sheet using installed ImageMagick
    #[arg(long)]
    pub sheet: bool,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Source {
    path: PathBuf,
    bytes: u64,
    modified_ns: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Frame {
    pub id: String,
    pub path: String,
    pub timestamp_seconds: f64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Sample {
    contract: String,
    source: Source,
    timeline: String,
    parent: Option<PathBuf>,
    round: usize,
    total_frames: usize,
    total_frame_budget: usize,
    requested: serde_json::Value,
    provenance: serde_json::Value,
    status: String,
    warnings: Vec<String>,
    frames: Vec<Frame>,
    sheet: Option<String>,
    elapsed_ms: u128,
}

fn validate(o: &SampleOptions) -> Result<(), String> {
    if !o.start.is_finite() || !o.end.is_finite() || o.start < 0.0 || o.end <= o.start {
        return Err("require finite seconds with 0 <= start < end".into());
    }
    if !o.fps.is_finite() || o.fps <= 0.0 || o.fps > 120.0 {
        return Err("fps must be finite and in (0, 120]".into());
    }
    if o.max_frames == 0 || o.max_frames > 1000 || o.total_frame_budget > 10000 {
        return Err("max-frames must be 1..1000; total-frame-budget must be <= 10000".into());
    }
    if (o.end - o.start) * o.fps > o.max_frames as f64 + 1e-8 {
        return Err(
            "requested window × fps exceeds max-frames; narrow the window or lower fps".into(),
        );
    }
    if !(32..=4096).contains(&o.max_size) || !(1..=600).contains(&o.timeout_seconds) {
        return Err("max-size must be 32..4096 and timeout-seconds 1..600".into());
    }
    crop_filter(o.crop.as_deref())?;
    Ok(())
}

fn crop_filter(crop: Option<&str>) -> Result<String, String> {
    let Some(crop) = crop else {
        return Ok(String::new());
    };
    let values: Vec<u32> = crop
        .split(',')
        .map(|v| v.parse::<u32>())
        .collect::<Result<_, _>>()
        .map_err(|_| "crop requires integer x,y,width,height")?;
    if values.len() != 4 || values[2] == 0 || values[3] == 0 {
        return Err("crop requires integer x,y,width,height with positive dimensions".into());
    }
    Ok(format!(
        "crop='if(lte({},iw),{},0)':'if(lte({},ih),{},0)':{}:{}:exact=1,",
        u64::from(values[0]) + u64::from(values[2]),
        values[2],
        u64::from(values[1]) + u64::from(values[3]),
        values[3],
        values[0],
        values[1]
    ))
}

fn source(path: &PathBuf) -> Result<Source, String> {
    let path = fs::canonicalize(path).map_err(|e| format!("local input: {e}"))?;
    let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("input must be a regular local video file".into());
    }
    Ok(Source {
        path,
        bytes: metadata.len(),
        modified_ns: metadata
            .modified()
            .map_err(|e| e.to_string())?
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos()
            .to_string(),
    })
}

/// Write subprocess output to disk instead of filling a pipe; kill and reap on timeout.
fn bounded(command: &mut Command, log: &PathBuf, deadline: Instant) -> Result<bool, String> {
    let stderr = fs::File::create(log).map_err(|e| e.to_string())?;
    if Instant::now() >= deadline {
        return Err("processing budget exhausted".into());
    }
    let mut child = command
        .stdin(Stdio::null())
        .stderr(stderr)
        .spawn()
        .map_err(|e| format!("start local tool: {e}"))?;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status.success()),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            result => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(match result {
                    Err(e) => format!("wait for local tool: {e}"),
                    _ => "processing budget exhausted; partial files are not a completed sample"
                        .into(),
                });
            }
        }
    }
}

fn timestamps(log: &str) -> Vec<f64> {
    log.lines()
        .filter(|line| line.contains("showinfo") && line.contains("pts_time:"))
        .filter_map(|line| {
            line.split("pts_time:")
                .nth(1)?
                .split_whitespace()
                .next()?
                .parse()
                .ok()
        })
        .collect()
}

pub fn execute(o: SampleOptions) -> Result<(), String> {
    validate(&o)?;
    let started = Instant::now();
    let deadline = started + Duration::from_secs(o.timeout_seconds);
    let source = source(&o.video)?;
    let parent = o
        .parent
        .as_ref()
        .map(fs::canonicalize)
        .transpose()
        .map_err(|e| e.to_string())?;
    let previous: Option<Sample> = parent
        .as_ref()
        .map(|path| {
            let bytes = fs::read(path).map_err(|e| e.to_string())?;
            serde_json::from_slice(&bytes).map_err(|e| format!("parent sample: {e}"))
        })
        .transpose()?;
    let (round, previous_count, budget) = if let Some(previous) = previous {
        if previous.contract != CONTRACT || previous.source != source {
            return Err("parent must be a sample of the same unchanged local file".into());
        }
        if previous.round >= 20 {
            return Err("maximum 20 linked rounds reached".into());
        }
        (
            previous.round + 1,
            previous.total_frames,
            previous.total_frame_budget.min(o.total_frame_budget),
        )
    } else {
        (1, 0, o.total_frame_budget)
    };
    let estimated = ((o.end - o.start) * o.fps).ceil() as usize;
    if previous_count
        .checked_add(estimated)
        .is_none_or(|count| count > budget)
    {
        return Err(
            "cumulative frame budget exhausted; reuse evidence or narrow the request".into(),
        );
    }

    if let Some(parent_dir) = o.out.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent_dir).map_err(|e| e.to_string())?;
    }
    fs::create_dir(&o.out).map_err(|e| format!("output must be a new directory: {e}"))?;
    let out = fs::canonicalize(&o.out).map_err(|e| e.to_string())?;
    let filter = format!(
        "setpts=PTS-STARTPTS,trim=start={}:end={},select='isnan(prev_selected_t)+gte(t-prev_selected_t,{})',{}scale='min({},iw)':'min({},ih)':force_original_aspect_ratio=decrease,showinfo",
        o.start,
        o.end,
        1.0 / o.fps,
        crop_filter(o.crop.as_deref())?,
        o.max_size,
        o.max_size,
    );
    let log_path = out.join("decode.log");
    let success = bounded(
        Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "info",
                "-n",
                "-protocol_whitelist",
                "file",
                "-format_whitelist",
                "mov,matroska,webm,avi,mpeg,mpegts,flv,ogg",
                "-i",
            ])
            .arg(&source.path)
            .args([
                "-map",
                "0:v:0",
                "-an",
                "-sn",
                "-dn",
                "-vf",
                &filter,
                "-frames:v",
                &o.max_frames.to_string(),
                "-fps_mode",
                "passthrough",
                "-q:v",
                "3",
            ])
            .arg(out.join("frame_%04d.jpg"))
            .stdout(Stdio::null()),
        &log_path,
        deadline,
    )?;
    let log = fs::read_to_string(&log_path).map_err(|e| e.to_string())?;
    let times = timestamps(&log);
    let frames: Vec<Frame> = times
        .into_iter()
        .enumerate()
        .filter_map(|(index, timestamp)| {
            let path = format!("frame_{:04}.jpg", index + 1);
            out.join(&path).is_file().then(|| Frame {
                id: format!("r{round}:f{}", index + 1),
                path,
                timestamp_seconds: timestamp,
            })
        })
        .collect();
    if !success || frames.is_empty() {
        return Err(format!(
            "decoding failed or window contains no frames; inspect {} (no sample.json published)",
            log_path.display()
        ));
    }
    let mut warnings = vec!["Sparse sampling cannot establish that unsampled events are absent. Extracted does not mean viewed by a model.".into()];
    let damaged = [
        "partial file",
        "Invalid data",
        "Error",
        "corrupt",
        "Invalid NAL",
    ]
    .iter()
    .any(|marker| log.contains(marker));
    if damaged {
        warnings.push("Decoder reported damaged or incomplete input; inspect decode.log. Do not claim full-window coverage.".into());
    }
    let mut sheet = None;
    if o.sheet {
        let mut montage = Command::new("magick");
        // A discovered font avoids assuming the same font package on Linux and macOS.
        let font_file = fs::File::create(out.join("fonts.txt")).map_err(|e| e.to_string())?;
        let _ = bounded(
            Command::new("magick")
                .args(["-list", "font"])
                .stdout(font_file),
            &out.join("fonts.log"),
            deadline,
        );
        let fonts = fs::read_to_string(out.join("fonts.txt")).unwrap_or_default();
        let font = fonts
            .lines()
            .find_map(|line| line.trim().strip_prefix("Font: "))
            .or_else(|| {
                // Some macOS ImageMagick builds do not enumerate installed fonts.
                [
                    "/System/Library/Fonts/Supplemental/Arial.ttf",
                    "/Library/Fonts/Arial Unicode.ttf",
                    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                ]
                .into_iter()
                .find(|path| std::path::Path::new(path).is_file())
            });
        if let Some(font) = font {
            montage.args([
                "montage",
                "-font",
                font,
                "-pointsize",
                "14",
                "-background",
                "#111827",
                "-fill",
                "white",
            ]);
            for frame in &frames {
                montage
                    .args([
                        "-label",
                        &format!("{} | {:.3}s", frame.id, frame.timestamp_seconds),
                    ])
                    .arg(out.join(&frame.path));
            }
            montage
                .args(["-thumbnail", "240x240", "-tile", "4x", "-geometry", "+6+6"])
                .arg(out.join("sheet.jpg"));
            match bounded(
                montage.stdout(Stdio::null()),
                &out.join("sheet.log"),
                deadline,
            ) {
                Ok(true) => sheet = Some("sheet.jpg".into()),
                _ => warnings
                    .push("Contact sheet failed; use the individual frames and sheet.log.".into()),
            }
        } else {
            warnings.push("No ImageMagick font available; use individual frames.".into());
        }
    }
    let sample = Sample {
        contract: CONTRACT.into(),
        source,
        timeline: "decoded video PTS normalized to first frame = 0; not frame index / nominal FPS"
            .into(),
        parent,
        round,
        total_frames: previous_count + frames.len(),
        total_frame_budget: budget,
        requested: serde_json::json!({"start_seconds":o.start,"end_seconds":o.end,"fps":o.fps,"max_frames":o.max_frames,"max_size":o.max_size,"crop":o.crop,"timeout_seconds":o.timeout_seconds}),
        provenance: serde_json::json!({"provider":"local-ffmpeg","locality":"local","model":null,"filter":filter,"decode_log":"decode.log"}),
        status: if damaged { "partial" } else { "sampled" }.into(),
        warnings,
        frames,
        sheet,
        elapsed_ms: started.elapsed().as_millis(),
    };
    let json = serde_json::to_string_pretty(&sample).map_err(|e| e.to_string())?;
    fs::write(out.join("sample.json"), &json).map_err(|e| e.to_string())?;
    println!("{json}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_pts_not_frame_index() {
        assert_eq!(
            timestamps(
                "[Parsed_showinfo] n: 0 pts: 25 pts_time:1.04167 s:540x960\n[Parsed_showinfo] n: 1 pts: 50 pts_time:2.08333"
            ),
            vec![1.04167, 2.08333]
        );
    }
    #[test]
    fn crop_rejects_filter_injection_and_empty_rectangles() {
        assert!(crop_filter(Some("0,0,10,10;movie=http://example.com")).is_err());
        assert!(crop_filter(Some("0,0,0,10")).is_err());
        assert_eq!(
            crop_filter(Some("10,20,100,200")).unwrap(),
            "crop='if(lte(110,iw),100,0)':'if(lte(220,ih),200,0)':10:20:exact=1,"
        );
    }
}
