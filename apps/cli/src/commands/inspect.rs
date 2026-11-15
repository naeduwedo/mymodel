//! `gguf inspect` - parse a GGUF file and explain what it is.
//!
//! Offline by design: this reads the file and nothing else. It is the tool an
//! issue should start with, because it answers "what did the parser actually see"
//! without trusting a file name, a repository or a download log.

use std::path::PathBuf;

use anyhow::Result;
use gguf_engine_gguf::{compatibility, manifest_from_gguf, parse_file};
use serde::Serialize;

use crate::output;

#[derive(Serialize)]
struct InspectReport {
    ok: bool,
    path: String,
    file_bytes: u64,
    version: u32,
    architecture: Option<String>,
    name: Option<String>,
    quantisation: String,
    alignment: u32,
    tensor_count: usize,
    metadata_count: usize,
    tensor_bytes: u64,
    tensor_types: Vec<(String, usize)>,
    block_count: Option<u64>,
    context_length: Option<u64>,
    embedding_length: Option<u64>,
    compatibility: String,
    compatibility_reason: String,
}

fn rejected(path: &std::path::Path, reason: &str) -> InspectReport {
    InspectReport {
        ok: false,
        path: path.display().to_string(),
        file_bytes: 0,
        version: 0,
        architecture: None,
        name: None,
        quantisation: String::new(),
        alignment: 0,
        tensor_count: 0,
        metadata_count: 0,
        tensor_bytes: 0,
        tensor_types: Vec::new(),
        block_count: None,
        context_length: None,
        embedding_length: None,
        compatibility: "rejected".to_owned(),
        compatibility_reason: reason.to_owned(),
    }
}

fn key(gguf: &gguf_engine_gguf::Gguf, architecture: Option<&str>, suffix: &str) -> Option<u64> {
    let arch = architecture?;
    gguf.metadata_u64(&format!("{arch}.{suffix}"))
}

fn accepted(path: &std::path::Path, gguf: &gguf_engine_gguf::Gguf) -> InspectReport {
    let architecture = gguf.architecture().map(str::to_owned);
    // Resolve the derived fields before moving `architecture` into the report.
    let block_count = key(gguf, architecture.as_deref(), "block_count");
    let context_length = key(gguf, architecture.as_deref(), "context_length");
    let embedding_length = key(gguf, architecture.as_deref(), "embedding_length");

    // No repository facts exist for a local file, so the manifest is local-only.
    let manifest = manifest_from_gguf(
        gguf,
        "local/file",
        "local",
        &path.display().to_string(),
        None,
        None,
    );
    let report = compatibility(&manifest, false);

    InspectReport {
        ok: true,
        path: path.display().to_string(),
        file_bytes: gguf.file_len,
        version: gguf.version,
        architecture,
        name: gguf.name().map(str::to_owned),
        quantisation: gguf.quantisation_label(),
        alignment: gguf.alignment,
        tensor_count: gguf.tensors.len(),
        metadata_count: gguf.metadata.len(),
        tensor_bytes: gguf.tensor_bytes,
        tensor_types: gguf.tensor_type_histogram().into_iter().collect(),
        block_count,
        context_length,
        embedding_length,
        compatibility: report.state.as_str().to_owned(),
        compatibility_reason: report.reason,
    }
}

/// Inspect one or more GGUF files. Returns false when any file was rejected.
pub fn inspect(paths: &[PathBuf], json: bool) -> Result<bool> {
    if paths.is_empty() {
        anyhow::bail!("no file given");
    }

    let mut all_ok = true;
    let mut reports = Vec::new();

    for path in paths {
        match parse_file(path) {
            Ok(gguf) => reports.push(accepted(path, &gguf)),
            Err(err) => {
                all_ok = false;
                reports.push(rejected(path, &err.to_string()));
            }
        }
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&reports)?);
        return Ok(all_ok);
    }

    for report in &reports {
        println!("{}", report.path);
        if !report.ok {
            println!("  REJECTED: {}", report.compatibility_reason);
            println!();
            continue;
        }

        println!("  format        : GGUF v{}", report.version);
        println!(
            "  architecture  : {}",
            report.architecture.as_deref().unwrap_or("(not declared)")
        );
        println!(
            "  name          : {}",
            report.name.as_deref().unwrap_or("(not declared)")
        );
        println!("  quantisation  : {}", report.quantisation);
        println!(
            "  file size     : {}",
            output::human_bytes(report.file_bytes)
        );
        println!(
            "  tensors       : {} over {}",
            report.tensor_count,
            output::human_bytes(report.tensor_bytes)
        );

        let types: Vec<String> = report
            .tensor_types
            .iter()
            .map(|(name, count)| format!("{name} x{count}"))
            .collect();
        println!("  tensor types  : {}", types.join(", "));
        println!(
            "  metadata      : {} entries, alignment {}",
            report.metadata_count, report.alignment
        );

        if let Some(blocks) = report.block_count {
            println!("  layers        : {blocks}");
        }
        if let Some(context) = report.context_length {
            println!("  context length: {context}");
        }
        if let Some(embedding) = report.embedding_length {
            println!("  embedding     : {embedding}");
        }

        println!("  compatibility : {}", report.compatibility);
        println!("                  {}", report.compatibility_reason);
        println!();
    }

    Ok(all_ok)
}
