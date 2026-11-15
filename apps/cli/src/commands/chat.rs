//! `gguf chat` - run a local streaming chat through llama.cpp.
//!
//! This is the end-to-end command of the V0.1 milestone (spec 18): download a
//! model, load it, and stream real tokens from real hardware.

use anyhow::{Context as _, Result};
use gguf_engine_runtime_llama_cpp::LlamaCppAdapter;
use gguf_engine_runtime_manager::{ChatMessage, ChatRequest, LoadPlan, RuntimeAdapter};
use tokio_stream::StreamExt as _;

/// Stream a chat completion from a local model.
pub async fn chat(model: &str, prompt: &str, max_tokens: u32, temperature: f32) -> Result<bool> {
    let adapter = LlamaCppAdapter::discover().context("locating llama.cpp")?;

    let caps = adapter.capabilities().await.context("probing llama.cpp")?;
    let gpu = caps.backends.iter().any(|b| b.as_str() == "cuda");
    eprintln!("runtime  : llama.cpp ({})", adapter.directory());
    eprintln!(
        "backends : {}",
        caps.backends
            .iter()
            .map(|b| b.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );

    let plan = LoadPlan {
        model_path: model.to_owned(),
        gpu_layers: if gpu { Some(99) } else { Some(0) },
        context_size: 2048,
        mode: gguf_engine_runtime_manager::LoadMode::Chat,
    };

    eprintln!("loading  : {model}");
    let instance = adapter.load(&plan).await.context("loading the model")?;
    eprintln!("loaded   : worker pid {:?}", instance.worker_pid);

    let request = ChatRequest {
        messages: vec![ChatMessage {
            role: "user".to_owned(),
            content: prompt.to_owned(),
        }],
        max_tokens,
        temperature,
        stream: true,
    };

    let mut stream = adapter
        .chat(&instance, &request)
        .await
        .context("starting inference")?;

    // Stream tokens to stdout as they arrive.
    let mut total = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("inference stream")?;
        print!("{}", chunk.content);
        use std::io::Write as _;
        std::io::stdout().flush().ok();
        total += 1;
    }
    println!();
    eprintln!("generated {total} chunk(s)");

    adapter.unload(instance).await.context("unloading")?;
    Ok(true)
}
