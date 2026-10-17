//! `video-perception catalog list` and `video-perception catalog search` subcommands.

use crate::registry::{REGISTRY, streaming_profile};

pub fn list(json: bool) -> Result<(), String> {
    if json {
        return list_json();
    }

    eprintln!(
        "{:<16} {:<48} {:<8} {:<10} {:<14} {:<6} DEPS",
        "ID", "DETECTS", "SCOPE", "BACKEND", "STREAM", "READY"
    );
    eprintln!("{}", "-".repeat(126));

    for sensor in REGISTRY {
        let deps = if sensor.deps.is_empty() {
            "\u{2014}".to_string()
        } else {
            sensor.deps.join(", ")
        };

        let streaming = streaming_profile(sensor.id);
        eprintln!(
            "{:<16} {:<48} {:<8} {:<10} {:<14} {:<6} {}",
            sensor.id,
            truncate(sensor.detects, 46),
            sensor.scope,
            sensor.backend,
            streaming.strategy,
            if streaming.implemented { "yes" } else { "no" },
            deps,
        );
    }

    eprintln!("\n{} capabilities registered.", REGISTRY.len());
    Ok(())
}

fn list_json() -> Result<(), String> {
    let entries: Vec<serde_json::Value> = REGISTRY
        .iter()
        .map(|s| {
            let streaming = streaming_profile(s.id);
            serde_json::json!({
                "id": s.id,
                "detects": s.detects,
                "scope": s.scope.to_string(),
                "backend": s.backend.to_string(),
                "deps": s.deps,
                "streaming": {
                    "strategy": streaming.strategy,
                    "implemented": streaming.implemented,
                    "providers": streaming.providers,
                    "default_window_ms": streaming.default_window_ms,
                    "expected_first_event_ms": streaming.expected_first_event_ms,
                    "parallelism": streaming.parallelism,
                    "notes": streaming.notes,
                },
            })
        })
        .collect();

    println!(
        "{}",
        serde_json::to_string_pretty(&entries).map_err(|e| format!("JSON: {e}"))?
    );
    Ok(())
}

pub fn search(query: &str, json: bool) -> Result<(), String> {
    let query_lower = query.to_lowercase();
    let query_words: Vec<&str> = query_lower.split_whitespace().collect();

    let mut scored: Vec<(&crate::registry::SensorDef, u32)> = REGISTRY
        .iter()
        .filter_map(|s| {
            let id_lower = s.id.to_lowercase();
            let detects_lower = s.detects.to_lowercase();

            let mut score: u32 = 0;

            if id_lower == query_lower {
                score += 100;
            } else if id_lower.contains(&query_lower) {
                score += 80;
            }

            if detects_lower.contains(&query_lower) {
                score += 60;
            }

            for word in &query_words {
                if word.len() >= 3 && detects_lower.contains(*word) {
                    score += 20;
                }
            }

            if score > 0 { Some((s, score)) } else { None }
        })
        .collect();

    scored.sort_by(|a, b| b.1.cmp(&a.1));

    if scored.is_empty() {
        eprintln!(
            "No capabilities match \"{query}\". Run `video-perception catalog list` to see all."
        );
        return Ok(());
    }

    if json {
        let entries: Vec<serde_json::Value> = scored
            .iter()
            .map(|(s, score)| {
                let streaming = streaming_profile(s.id);
                serde_json::json!({
                    "id": s.id,
                    "detects": s.detects,
                    "score": score,
                    "scope": s.scope.to_string(),
                    "backend": s.backend.to_string(),
                    "streaming": {
                        "strategy": streaming.strategy,
                        "implemented": streaming.implemented,
                        "providers": streaming.providers,
                    },
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&entries).map_err(|e| format!("JSON: {e}"))?
        );
        return Ok(());
    }

    eprintln!("Sensors matching \"{}\":\n", query);
    eprintln!(
        "{:<16} {:<62} {:<8} {}",
        "ID", "DETECTS", "SCOPE", "BACKEND"
    );
    eprintln!("{}", "-".repeat(100));

    for (sensor, _score) in &scored {
        eprintln!(
            "{:<16} {:<62} {:<8} {}",
            sensor.id,
            truncate(sensor.detects, 60),
            sensor.scope,
            sensor.backend,
        );
    }

    eprintln!("\n{} match(es).", scored.len());
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let end = s
        .char_indices()
        .nth(max - 3)
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    format!("{}...", &s[..end])
}
