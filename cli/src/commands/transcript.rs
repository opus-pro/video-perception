use crate::provider::{
    self, CapabilityProvider, ExecutionPolicy, Locality, ProviderAttempt, ProviderDescriptor,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Deserialize, Serialize)]
pub struct Word {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Utterance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub words: Option<Vec<Word>>,
}

#[derive(Debug, Deserialize, Serialize)]
struct TranscriptPayload {
    #[serde(default = "unknown_language")]
    language: String,
    utterances: Vec<Utterance>,
}

#[derive(Serialize)]
struct TranscriptProvenance {
    provider: String,
    locality: Locality,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    timing: String,
    attempts: Vec<ProviderAttempt>,
}

#[derive(Serialize)]
pub struct Transcript {
    provenance: TranscriptProvenance,
    pub language: String,
    pub utterances: Vec<Utterance>,
}

struct TranscriptRequest {
    video: String,
    lang: Option<String>,
}

struct AppleSpeechProvider;
struct WhisperCppProvider;

pub(crate) fn doctor_status() -> Result<String, String> {
    let requested_provider =
        nonempty_env("VIDEO_PERCEPTION_TRANSCRIPT_PROVIDER").unwrap_or_else(|| "auto".to_string());
    let policy_value = nonempty_env("VIDEO_PERCEPTION_TRANSCRIPT_POLICY")
        .unwrap_or_else(|| "local-only".to_string());
    let policy = ExecutionPolicy::parse(&policy_value)?;

    let apple = AppleSpeechProvider;
    let whisper = WhisperCppProvider;
    let providers: [&dyn CapabilityProvider<Request = TranscriptRequest, Output = TranscriptPayload>;
        2] = [&apple, &whisper];
    let resolution = provider::resolve_available(&providers, &requested_provider, policy)
        .map_err(|error| error.to_string())?;

    Ok(format!(
        "selected: {} ({}; policy: {policy_value}; checked: {})",
        resolution.provider.id,
        resolution.provider.locality.as_str(),
        resolution.attempts.len()
    ))
}

pub fn execute(
    video: &str,
    lang: Option<&str>,
    requested_provider: Option<&str>,
    requested_policy: Option<&str>,
) -> Result<(), String> {
    if !tool_exists("ffmpeg") || !tool_exists("ffprobe") {
        return Err("transcript requires ffmpeg and ffprobe in PATH".to_string());
    }

    let requested_provider = requested_provider
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| nonempty_env("VIDEO_PERCEPTION_TRANSCRIPT_PROVIDER"))
        .unwrap_or_else(|| "auto".to_string());
    let policy_value = requested_policy
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| nonempty_env("VIDEO_PERCEPTION_TRANSCRIPT_POLICY"))
        .unwrap_or_else(|| "local-only".to_string());
    let policy = ExecutionPolicy::parse(&policy_value)?;

    let apple = AppleSpeechProvider;
    let whisper = WhisperCppProvider;
    let providers: [&dyn CapabilityProvider<Request = TranscriptRequest, Output = TranscriptPayload>;
        2] = [&apple, &whisper];
    let request = TranscriptRequest {
        video: video.to_string(),
        lang: lang.map(str::to_owned),
    };
    let resolution = provider::resolve(&providers, &request, &requested_provider, policy)
        .map_err(|error| format!("transcript provider resolution failed: {error}"))?;

    for attempt in &resolution.attempts {
        if let Some(reason) = &attempt.reason {
            eprintln!("  transcript provider {}: {reason}", attempt.provider);
        }
    }
    let timing = resolution.provider.timing.unwrap_or("unknown").to_string();
    let transcript = Transcript {
        provenance: TranscriptProvenance {
            provider: resolution.provider.id.to_string(),
            locality: resolution.provider.locality,
            model: resolution.provider.model,
            timing,
            attempts: resolution.attempts,
        },
        language: resolution.output.language,
        utterances: resolution.output.utterances,
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&transcript)
            .map_err(|e| format!("Could not encode transcript JSON: {e}"))?
    );
    Ok(())
}

impl CapabilityProvider for AppleSpeechProvider {
    type Request = TranscriptRequest;
    type Output = TranscriptPayload;

    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: "apple-speech",
            locality: Locality::Local,
            model: Some("Speech.framework".to_string()),
            timing: Some("aligned"),
        }
    }

    fn availability(&self) -> Result<(), String> {
        if !cfg!(target_os = "macos") {
            return Err("only available on macOS".to_string());
        }
        let bin = apple_speech_bin();
        if tool_exists(&bin) || Path::new(&bin).is_file() {
            Ok(())
        } else {
            Err(format!(
                "{bin} not found; install the Apple provider host or set VIDEO_PERCEPTION_APPLE_SPEECH_BIN"
            ))
        }
    }

    fn execute(&self, request: &TranscriptRequest) -> Result<TranscriptPayload, String> {
        let mut command = Command::new(apple_speech_bin());
        command.arg("transcript").arg("--input").arg(&request.video);
        if let Some(lang) = request.lang.as_deref() {
            command.arg("--language").arg(lang);
        }
        let output = command
            .output()
            .map_err(|e| format!("Apple Speech provider execution failed: {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "Apple Speech provider exited with {} ({})",
                output.status,
                stderr_tail(&output.stderr)
            ));
        }
        serde_json::from_slice(&output.stdout)
            .map_err(|e| format!("Apple Speech provider returned invalid JSON: {e}"))
    }
}

impl CapabilityProvider for WhisperCppProvider {
    type Request = TranscriptRequest;
    type Output = TranscriptPayload;

    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: "whisper-cpp",
            locality: Locality::Local,
            model: nonempty_env("WHISPER_CPP_MODEL"),
            timing: Some("aligned"),
        }
    }

    fn availability(&self) -> Result<(), String> {
        let bin = nonempty_env("WHISPER_CPP_BIN").ok_or("WHISPER_CPP_BIN not set")?;
        let model = nonempty_env("WHISPER_CPP_MODEL").ok_or("WHISPER_CPP_MODEL not set")?;
        if !tool_exists(&bin) && !Path::new(&bin).is_file() {
            return Err(format!("whisper.cpp binary not found: {bin}"));
        }
        if !Path::new(&model).is_file() {
            return Err(format!("whisper.cpp model not found: {model}"));
        }
        Ok(())
    }

    fn execute(&self, request: &TranscriptRequest) -> Result<TranscriptPayload, String> {
        whisper_from_env(&request.video, request.lang.as_deref())
    }
}

fn apple_speech_bin() -> String {
    nonempty_env("VIDEO_PERCEPTION_APPLE_SPEECH_BIN")
        .unwrap_or_else(|| "video-perception-apple".to_string())
}

fn unknown_language() -> String {
    "und".to_string()
}

fn nonempty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn tool_exists(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn whisper_from_env(video: &str, lang: Option<&str>) -> Result<TranscriptPayload, String> {
    let bin = nonempty_env("WHISPER_CPP_BIN").ok_or("WHISPER_CPP_BIN not set")?;
    let model = nonempty_env("WHISPER_CPP_MODEL").ok_or("WHISPER_CPP_MODEL not set")?;

    // Every intermediate file lives in a directory only this user can reach,
    // so a predictable name in the shared temp directory cannot be planted.
    let scratch = crate::temp::PrivateDir::create("transcript")?;
    let wav_path = scratch.file("audio.wav");
    extract_audio_wav(video, &wav_path)?;

    let json_prefix = scratch.file("whisper");
    let mut command = Command::new(&bin);
    command
        .arg("-m")
        .arg(&model)
        .arg("-f")
        .arg(&wav_path)
        .arg("-oj")
        .arg("-of")
        .arg(&json_prefix);
    if let Some(code) = lang {
        command.arg("-l").arg(code);
    }
    let output = command
        .output()
        .map_err(|e| format!("whisper.cpp execution failed: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "whisper.cpp exited with {} ({})",
            output.status,
            stderr_tail(&output.stderr)
        ));
    }

    let json_path = scratch.file("whisper.json");
    let payload = std::fs::read_to_string(&json_path)
        .map_err(|e| format!("Could not read whisper output {}: {e}", json_path.display()))?;
    parse_whisper_json(&payload)
}

fn parse_whisper_json(payload: &str) -> Result<TranscriptPayload, String> {
    let root: Value =
        serde_json::from_str(payload).map_err(|e| format!("Invalid whisper JSON: {e}"))?;

    let language = root
        .get("result")
        .and_then(|v| v.get("language"))
        .and_then(Value::as_str)
        .or_else(|| root.get("language").and_then(Value::as_str))
        .unwrap_or("und")
        .to_string();

    let utterances = if let Some(arr) = root.get("transcription").and_then(Value::as_array) {
        parse_whisper_transcription_rows(arr)
    } else if let Some(arr) = root.get("segments").and_then(Value::as_array) {
        parse_whisper_segment_rows(arr)
    } else {
        Vec::new()
    };

    Ok(TranscriptPayload {
        language,
        utterances,
    })
}

fn parse_whisper_transcription_rows(rows: &[Value]) -> Vec<Utterance> {
    rows.iter()
        .filter_map(|row| {
            let start_ms = row
                .get("offsets")
                .and_then(|o| o.get("from"))
                .and_then(Value::as_u64)
                .or_else(|| {
                    row.get("timestamps")
                        .and_then(|t| t.get("from"))
                        .and_then(Value::as_str)
                        .and_then(parse_timestamp_ms)
                })?;
            let mut end_ms = row
                .get("offsets")
                .and_then(|o| o.get("to"))
                .and_then(Value::as_u64)
                .or_else(|| {
                    row.get("timestamps")
                        .and_then(|t| t.get("to"))
                        .and_then(Value::as_str)
                        .and_then(parse_timestamp_ms)
                })
                .unwrap_or(start_ms + 1);
            if end_ms <= start_ms {
                end_ms = start_ms + 1;
            }
            let text = row
                .get("text")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())?
                .to_owned();

            Some(Utterance {
                speaker: None,
                text,
                start_ms,
                end_ms,
                words: None,
            })
        })
        .collect()
}

fn parse_whisper_segment_rows(rows: &[Value]) -> Vec<Utterance> {
    rows.iter()
        .filter_map(|row| {
            let start_ms = row
                .get("t0")
                .and_then(Value::as_u64)
                .or_else(|| row.get("start_ms").and_then(Value::as_u64))?;
            let mut end_ms = row
                .get("t1")
                .and_then(Value::as_u64)
                .or_else(|| row.get("end_ms").and_then(Value::as_u64))
                .unwrap_or(start_ms + 1);
            if end_ms <= start_ms {
                end_ms = start_ms + 1;
            }
            let text = row
                .get("text")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())?
                .to_owned();

            Some(Utterance {
                speaker: None,
                text,
                start_ms,
                end_ms,
                words: None,
            })
        })
        .collect()
}

fn extract_audio_wav(video: &str, wav_path: &Path) -> Result<(), String> {
    let output = Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            video,
            "-vn",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-c:a",
            "pcm_s16le",
        ])
        .arg(wav_path)
        .output()
        .map_err(|e| format!("ffmpeg audio extraction failed: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "ffmpeg audio extraction exited with {} ({})",
            output.status,
            stderr_tail(&output.stderr)
        ))
    }
}

fn parse_timestamp_ms(ts: &str) -> Option<u64> {
    let normalized = ts.replace(',', ".");
    let parts = normalized.split(':').collect::<Vec<_>>();
    if parts.len() != 3 {
        return None;
    }
    let hours = parts[0].parse::<u64>().ok()?;
    let minutes = parts[1].parse::<u64>().ok()?;
    let seconds = parts[2].parse::<f64>().ok()?;
    let total_ms = ((hours * 3600 + minutes * 60) as f64 + seconds) * 1000.0;
    Some(total_ms.round() as u64)
}

fn stderr_tail(stderr: &[u8]) -> String {
    text_tail(&String::from_utf8_lossy(stderr), 500)
}

fn text_tail(text: &str, max_chars: usize) -> String {
    let chars = text.trim().chars().collect::<Vec<_>>();
    if chars.len() <= max_chars {
        return chars.into_iter().collect();
    }
    chars[chars.len() - max_chars..].iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_current_whisper_json() {
        let payload = parse_whisper_json(
            r#"{"result":{"language":"zh"},"transcription":[{"offsets":{"from":0,"to":500},"text":" 你好 "}]}"#,
        )
        .unwrap();

        assert_eq!(payload.language, "zh");
        assert_eq!(payload.utterances[0].text, "你好");
        assert_eq!(payload.utterances[0].end_ms, 500);
    }
}
