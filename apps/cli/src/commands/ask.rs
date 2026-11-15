//! `gguf ask` - chat through the running core over gRPC.
//!
//! This is deliberately different from `gguf chat`, which loads a model in-process.
//! `ask` exercises the same path the native UI uses: connect to the core, send a
//! `ChatRequest`, render the token stream. If this works, the UI's transport
//! works.

use anyhow::{Context as _, Result};
use gguf_engine_protocol::pb;
use tokio_stream::StreamExt;

use crate::commands::Context;

/// Ask the running core to complete a prompt.
pub async fn ask(
    ctx: &Context,
    model: &str,
    prompt: &str,
    system: Option<&str>,
    max_tokens: u32,
    temperature: f32,
    json: bool,
) -> Result<bool> {
    let client = gguf_engine_ipc::client::connect(ctx.paths.root())
        .await
        .context("connecting to the core; start it with `gguf serve`")?;

    let mut messages = Vec::new();
    if let Some(system) = system {
        messages.push(pb::ChatMessage {
            role: "system".to_owned(),
            content: system.to_owned(),
        });
    }
    messages.push(pb::ChatMessage {
        role: "user".to_owned(),
        content: prompt.to_owned(),
    });

    let request = pb::ChatRequest {
        model_id: model.to_owned(),
        messages,
        task_id: None,
        max_tokens: Some(max_tokens),
        temperature: Some(temperature),
        stream: Some(true),
    };

    let mut stream = client.chat(request).await.context("starting the chat")?;

    let mut answer = String::new();
    let mut chunks = 0u32;
    let mut final_progress = None;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("reading a chat chunk")?;
        if !chunk.delta.is_empty() {
            chunks += 1;
            answer.push_str(&chunk.delta);
            if !json {
                print!("{}", chunk.delta);
                // Flush so the terminal shows tokens as they arrive rather than
                // in one burst when the stream ends.
                use std::io::Write as _;
                let _ = std::io::stdout().flush();
            }
        }
        if chunk.done {
            final_progress = chunk.progress;
            break;
        }
    }

    if json {
        let progress = final_progress.unwrap_or_default();
        println!(
            "{}",
            serde_json::json!({
                "ok": !answer.is_empty(),
                "answer": answer,
                "chunks": chunks,
                "tokens_generated": progress.tokens_generated,
                "elapsed_ms": progress.elapsed_ms,
                "tokens_per_second": progress.tokens_per_second,
            })
        );
    } else {
        println!();
        if let Some(progress) = final_progress {
            let rate = progress
                .tokens_per_second
                .map(|rate| format!("{rate:.1} tok/s"))
                .unwrap_or_else(|| "rate unavailable".to_owned());
            eprintln!(
                "[{} chunks, {} tokens, {} ms, {}]",
                chunks, progress.tokens_generated, progress.elapsed_ms, rate
            );
        }
    }

    Ok(!answer.is_empty())
}
