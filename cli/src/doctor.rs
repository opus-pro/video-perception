/// `video-perception doctor` — read-only live capability audit.
///
/// Checks core tools and user-managed local model paths.
use crate::util;

pub fn execute(json: bool) -> Result<(), String> {
    let groups = build_groups();

    if json {
        println!("{}", format_json(&groups)?);
        return Ok(());
    }

    print_table(&groups);

    let missing = core_missing_names(&groups);
    if !missing.is_empty() {
        return Err(format!("Missing core deps: {}", missing.join(", ")));
    }

    Ok(())
}

// ── Types ──────────────────────────────────────────────────

struct Group {
    name: &'static str,
    items: Vec<Item>,
}

struct Item {
    name: String,
    ok: bool,
    version: Option<String>,
    fix: Option<String>,
}

// ── Group construction ────────────────────────────────────

fn build_groups() -> Vec<Group> {
    let mut groups = vec![
        Group {
            name: "Core (required)",
            items: vec![
                check_tool("ffmpeg", "brew install ffmpeg"),
                check_tool("ffprobe", "brew install ffmpeg"),
                check_tool("magick", "brew install imagemagick"),
            ],
        },
        Group {
            name: "Default pipeline (best-effort)",
            items: vec![
                check_tool(
                    "autobeat",
                    "uv tool install --python 3.11 tools/autobeat  (from repo root)",
                ),
                check_tool(
                    "video-perception-spectral",
                    "uv tool install --python 3.11 tools/spectral  (from repo root)",
                ),
                check_tool(
                    "video-perception-audio-class",
                    "uv tool install --python 3.11 tools/audio-class  (from repo root)",
                ),
                check_tool(
                    "video-perception-motion-snn",
                    "uv tool install --python 3.11 tools/motion-snn  (from repo root)",
                ),
                check_transcript(),
            ],
        },
    ];

    groups.push(Group {
        name: "Model runner",
        items: vec![check_tool(
            "video-perception-vlm",
            "install the packaged video-perception-vlm adapter",
        )],
    });
    groups.push(Group {
        name: "Local model route (optional, user-managed)",
        items: vec![
            check_tool("ollama", "install Ollama explicitly"),
            check_ollama_model(),
        ],
    });

    groups.push(Group {
        name: "Setup",
        items: vec![check_tool("uv", "brew install uv")],
    });

    groups
}

// ── Checks ─────────────────────────────────────────────────

fn check_tool(name: &str, fix: &str) -> Item {
    let version = util::get_version(name);
    Item {
        name: name.to_string(),
        ok: version.is_some(),
        version,
        fix: Some(fix.to_string()),
    }
}

fn check_transcript() -> Item {
    match crate::commands::transcript::doctor_status() {
        Ok(status) => Item {
            name: "transcript".into(),
            ok: true,
            version: Some(status),
            fix: None,
        },
        Err(reason) => Item {
            name: "transcript".into(),
            ok: false,
            version: Some(reason),
            fix: Some("install or configure an eligible transcript provider".into()),
        },
    }
}

fn check_ollama_model() -> Item {
    let model = std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| "gemma4".into());
    let url = std::env::var("OLLAMA_URL").unwrap_or_else(|_| "http://localhost:11434".into());

    let result = std::process::Command::new("curl")
        .args(["-s", "--connect-timeout", "2", &format!("{url}/api/tags")])
        .output();

    match result {
        Ok(output) if output.status.success() => {
            let body = String::from_utf8_lossy(&output.stdout);
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&body) {
                let models = val.get("models").and_then(|m| m.as_array());
                if let Some(models) = models {
                    for m in models {
                        let name = m.get("name").and_then(|n| n.as_str()).unwrap_or("");
                        if name.starts_with(&model) {
                            let size = m.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
                            let size_gb = size as f64 / 1_073_741_824.0;
                            return Item {
                                name: model.clone(),
                                ok: true,
                                version: Some(format!("loaded ({:.1} GB)", size_gb)),
                                fix: None,
                            };
                        }
                    }
                    return Item {
                        name: model.clone(),
                        ok: false,
                        version: Some("not pulled".into()),
                        fix: Some(format!("ollama pull {model}")),
                    };
                }
            }
            Item {
                name: model,
                ok: false,
                version: Some("ollama API error".into()),
                fix: Some("brew services start ollama".into()),
            }
        }
        _ => Item {
            name: model,
            ok: false,
            version: Some("ollama not reachable".into()),
            fix: Some("brew services start ollama".into()),
        },
    }
}

// ── Output ─────────────────────────────────────────────────

fn print_table(groups: &[Group]) {
    for group in groups {
        eprintln!("\n{}:", group.name);
        for item in &group.items {
            let mark = if item.ok { "\u{2713}" } else { "\u{00b7}" };
            let status = item.version.as_deref().unwrap_or("not found");
            let fix = if item.ok {
                String::new()
            } else {
                item.fix
                    .as_ref()
                    .map(|f| format!(" \u{2192} {f}"))
                    .unwrap_or_default()
            };
            eprintln!("  {mark} {:<20} {}{}", item.name, status, fix);
        }
    }

    let total = groups.iter().flat_map(|g| &g.items).count();
    let ok = groups
        .iter()
        .flat_map(|g| &g.items)
        .filter(|i| i.ok)
        .count();
    eprintln!("\n{ok}/{total} capabilities available.");
}

fn format_json(groups: &[Group]) -> Result<String, String> {
    let json_groups: Vec<serde_json::Value> = groups
        .iter()
        .map(|g| {
            let items: Vec<serde_json::Value> = g
                .items
                .iter()
                .map(|i| {
                    let mut obj = serde_json::json!({
                        "name": i.name,
                        "ok": i.ok,
                    });
                    if let Some(v) = &i.version {
                        obj["version"] = serde_json::Value::String(v.clone());
                    }
                    if let Some(f) = &i.fix {
                        if !i.ok {
                            obj["fix"] = serde_json::Value::String(f.clone());
                        }
                    }
                    obj
                })
                .collect();
            serde_json::json!({
                "group": g.name,
                "items": items,
            })
        })
        .collect();

    serde_json::to_string_pretty(&json_groups).map_err(|e| format!("JSON: {e}"))
}

fn core_missing_names(groups: &[Group]) -> Vec<&str> {
    groups
        .first()
        .map(|g| {
            g.items
                .iter()
                .filter(|i| !i.ok)
                .map(|i| i.name.as_str())
                .collect()
        })
        .unwrap_or_default()
}

// ── Tests ──────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_item(name: &str, ok: bool, version: Option<&str>, fix: Option<&str>) -> Item {
        Item {
            name: name.to_string(),
            ok,
            version: version.map(String::from),
            fix: fix.map(String::from),
        }
    }

    #[test]
    fn json_output_schema() {
        let groups = vec![
            Group {
                name: "Core (required)",
                items: vec![
                    fake_item("ffmpeg", true, Some("6.1"), None),
                    fake_item("ffprobe", false, None, Some("brew install ffmpeg")),
                ],
            },
            Group {
                name: "Setup",
                items: vec![fake_item("uv", true, Some("0.4"), None)],
            },
        ];
        let json_str = format_json(&groups).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["group"], "Core (required)");

        let items = arr[0]["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);

        // ok item: has version, no fix key
        assert_eq!(items[0]["name"], "ffmpeg");
        assert_eq!(items[0]["ok"], true);
        assert_eq!(items[0]["version"], "6.1");
        assert!(items[0].get("fix").is_none());

        // not-ok item: has fix key, no version
        assert_eq!(items[1]["name"], "ffprobe");
        assert_eq!(items[1]["ok"], false);
        assert!(items[1].get("fix").is_some());
    }

    #[test]
    fn core_missing_detected() {
        let groups = vec![Group {
            name: "Core (required)",
            items: vec![
                fake_item("ffmpeg", false, None, Some("brew install ffmpeg")),
                fake_item("ffprobe", true, Some("6.1"), None),
                fake_item("magick", false, None, Some("brew install imagemagick")),
            ],
        }];
        let missing = core_missing_names(&groups);
        assert_eq!(missing, vec!["ffmpeg", "magick"]);
    }

    #[test]
    fn core_all_present() {
        let groups = vec![Group {
            name: "Core (required)",
            items: vec![
                fake_item("ffmpeg", true, Some("6.1"), None),
                fake_item("ffprobe", true, Some("6.1"), None),
                fake_item("magick", true, Some("7.1"), None),
            ],
        }];
        let missing = core_missing_names(&groups);
        assert!(missing.is_empty());
    }

    #[test]
    fn fix_messages_have_python_version() {
        let groups = build_groups();
        let pipeline = &groups[1]; // "Default pipeline (best-effort)"
        for item in &pipeline.items {
            if let Some(fix) = &item.fix {
                if fix.contains("uv tool install") {
                    assert!(
                        fix.contains("--python 3.11"),
                        "Fix for {} missing --python 3.11: {}",
                        item.name,
                        fix
                    );
                }
            }
        }
    }
}
