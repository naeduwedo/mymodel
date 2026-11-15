//! `gguf modalities` - what this build can actually do (spec 4.6, Task 8).
//!
//! Prints every modality with its status and the reason. Unimplemented ones are
//! listed, not hidden: a user asking "can it do audio?" deserves an answer.

use anyhow::Result;
use gguf_engine_runtime_manager::{ModalityStatus, modalities};
use serde::Serialize;

#[derive(Debug, Serialize)]
struct ModalityRow {
    modality: String,
    status: String,
    runtime: Option<String>,
    detail: String,
}

/// Print the modality matrix.
pub fn modalities_cmd(json: bool) -> Result<bool> {
    let rows: Vec<ModalityRow> = modalities()
        .into_iter()
        .map(|entry| ModalityRow {
            modality: entry.modality.as_str().to_owned(),
            status: entry.status.as_str().to_owned(),
            runtime: entry.runtime,
            detail: entry.detail,
        })
        .collect();

    if json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(true);
    }

    println!(
        "{:<18} {:<14} {:<22} detail",
        "modality", "status", "runtime"
    );
    for row in &rows {
        println!(
            "{:<18} {:<14} {:<22} {}",
            row.modality,
            row.status,
            row.runtime.as_deref().unwrap_or("-"),
            row.detail
        );
    }

    let supported = rows
        .iter()
        .filter(|row| row.status == ModalityStatus::Supported.as_str())
        .count();
    println!();
    println!(
        "{} of {} modalities are implemented in this build",
        supported,
        rows.len()
    );

    Ok(true)
}
