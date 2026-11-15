//! `gguf overview` - render the same snapshot the native UIs display.
//!
//! This is not a UI: it is the reference renderer that proves the contract works
//! end to end, and it is what support can paste into an issue. It renders only
//! what the snapshot contains, so a missing field shows up here before it shows
//! up in three separate shells.

use anyhow::{Context as _, Result};
use gguf_engine_protocol::pb;

use crate::commands::Context;

// Indexed by the UiSeverity enum value (0 = unspecified .. 4 = disabled).
const SEVERITY_WORD: [&str; 5] = ["?", "ok", "warning", "error", "n/a"];
const SEVERITY_BULLET: [&str; 5] = ["[?   ]", "[ok  ]", "[warn]", "[FAIL]", "[ n/a]"];

fn word(severity: i32) -> &'static str {
    SEVERITY_WORD.get(severity as usize).copied().unwrap_or("?")
}

fn bullet(severity: i32) -> &'static str {
    SEVERITY_BULLET
        .get(severity as usize)
        .copied()
        .unwrap_or("[?   ]")
}

/// Fetch and print the snapshot. Returns false when the core is not running.
pub async fn overview(ctx: &Context, json: bool) -> Result<bool> {
    let client = match gguf_engine_ipc::connect(ctx.paths.root()).await {
        Ok(client) => client,
        Err(gguf_engine_ipc::IpcError::NotRunning(endpoint)) => {
            if json {
                let payload = serde_json::json!({
                    "ok": false,
                    "reason": "core is not running",
                    "endpoint": endpoint,
                });
                println!("{payload}");
            } else {
                println!("core is not running (endpoint {endpoint})");
                println!("start it with: gguf-core --data-dir <DIR>");
            }
            return Ok(false);
        }
        Err(err) => return Err(err).context("connecting to the core"),
    };

    let snapshot = client
        .snapshot()
        .await
        .context("fetching the UI snapshot")?;

    if json {
        let text = serde_json::to_string_pretty(&json_view(&snapshot))?;
        println!("{text}");
        return Ok(true);
    }

    println!(
        "GGUF AI Engine  {}   snapshot v{}",
        snapshot.generated_at_local, snapshot.snapshot_version
    );

    if let Some(overview) = &snapshot.overview {
        println!();
        println!("OVERVIEW");
        for tile in &overview.tiles {
            println!(
                "  {} {:<18} {}",
                bullet(tile.severity),
                tile.label,
                tile.value
            );
            if !tile.detail.is_empty() {
                println!("        {:<18} {}", "", tile.detail);
            }
        }

        println!();
        println!("CAPABILITIES");
        for row in &overview.features {
            let reason = if row.reason.is_empty() {
                String::new()
            } else {
                format!("  ({})", row.reason)
            };
            println!(
                "  {} {:<24} {:<10}{}",
                bullet(row.severity),
                row.label,
                row.status_text,
                reason
            );
        }

        if !overview.recent_events.is_empty() {
            println!();
            println!("RECENT EVENTS");
            for line in &overview.recent_events {
                println!(
                    "  {} {} {}",
                    bullet(line.severity),
                    line.at_local,
                    line.text
                );
            }
        }

        if overview.attention_required {
            println!();
            println!("note: something above needs attention");
        }
    }

    if let Some(settings) = &snapshot.settings {
        println!();
        println!(
            "SETTINGS   config: {} ({})",
            settings.config_file,
            if settings.config_file_exists {
                "file in use"
            } else {
                "not found, defaults in use"
            }
        );
        for group in &settings.groups {
            println!();
            println!("  [{}]", group.title);
            for row in &group.rows {
                println!(
                    "    {} {:<28} {}",
                    bullet(row.severity),
                    row.label,
                    row.value
                );
                if !row.note.is_empty() {
                    println!("         {:<28} {}", "", row.note);
                }
            }
        }
    }

    Ok(true)
}

/// A stable JSON shape for tests and tooling.
fn json_view(snapshot: &pb::UiSnapshot) -> serde_json::Value {
    let overview = snapshot.overview.clone().unwrap_or_default();
    let settings = snapshot.settings.clone().unwrap_or_default();
    serde_json::json!({
        "ok": true,
        "snapshot_version": snapshot.snapshot_version,
        "generated_at": snapshot.generated_at_local,
        "overview": {
            "attention_required": overview.attention_required,
            "tiles": overview.tiles.iter().map(|tile| serde_json::json!({
                "id": tile.id,
                "label": tile.label,
                "value": tile.value,
                "severity": word(tile.severity),
                "detail": tile.detail,
            })).collect::<Vec<_>>(),
            "features": overview.features.iter().map(|row| serde_json::json!({
                "id": row.id,
                "label": row.label,
                "status": row.status_text,
                "reason": row.reason,
            })).collect::<Vec<_>>(),
            "events": overview.recent_events.iter().map(|line| serde_json::json!({
                "seq": line.seq,
                "at": line.at_local,
                "severity": word(line.severity),
                "text": line.text,
            })).collect::<Vec<_>>(),
        },
        "settings": {
            "config_file": settings.config_file,
            "config_file_exists": settings.config_file_exists,
            "groups": settings.groups.iter().map(|group| serde_json::json!({
                "title": group.title,
                "rows": group.rows.iter().map(|row| serde_json::json!({
                    "key": row.key,
                    "label": row.label,
                    "value": row.value,
                    "editable": row.editable,
                    "note": row.note,
                    "severity": word(row.severity),
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "issues": settings.issues.iter().map(|issue| serde_json::json!({
                "field": issue.field,
                "severity": word(issue.severity),
                "message": issue.message,
            })).collect::<Vec<_>>(),
        },
    })
}
