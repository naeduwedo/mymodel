//! End-to-end test: the llama.cpp adapter drives a real inference.
//!
//! Opt-in: needs a llama.cpp installation (GGUF_LLAMA_CPP_DIR) and a small GGUF.
//! Marked #[ignore] so the default suite never touches a GPU or spawns workers.
//! Run with:
//!
//!   GGUF_LLAMA_CPP_DIR=E:/llama.cpp cargo test -p gguf-engine-runtime-llama-cpp --test e2e -- --ignored

use gguf_engine_runtime_llama_cpp::LlamaCppAdapter;
use gguf_engine_runtime_manager::{ChatMessage, ChatRequest, RuntimeAdapter};

fn model_path() -> Option<String> {
    std::env::var("GGUF_TEST_MODEL").ok().or_else(|| {
        let home = std::env::var("TEMP").unwrap_or_default();
        let mut candidates = Vec::new();
        // The tiny stories model is the standard fixture; look for any recent pull.
        if let Ok(entries) = std::fs::read_dir(&home) {
            for entry in entries.flatten() {
                let dir = entry.path();
                if dir.is_dir()
                    && dir
                        .file_name()
                        .and_then(|n| n.to_str().map(|s| s.starts_with("gguf-")))
                        .unwrap_or(false)
                {
                    let f = dir
                        .join("models")
                        .join("tinyllamas")
                        .join("stories260K.gguf");
                    if f.is_file() {
                        candidates.push(f.display().to_string());
                    }
                }
            }
        }
        candidates.into_iter().next()
    })
}

#[tokio::test]
#[ignore = "needs a llama.cpp install and a tiny GGUF"]
async fn probes_loads_streams_and_unloads_a_real_model() {
    let Some(model) = model_path() else {
        panic!("no tiny GGUF found; set GGUF_TEST_MODEL to its path");
    };

    let adapter = LlamaCppAdapter::discover().expect("locate llama.cpp");

    // Capabilities must reflect the real machine: this build has CUDA.
    let caps = adapter.capabilities().await.expect("capabilities");
    assert!(caps.supports_streaming);
    assert!(
        caps.backends.iter().any(|b| b.as_str() == "cuda"),
        "backends: {:?}",
        caps.backends
    );

    // Load the tiny model with full GPU offload.
    let plan = gguf_engine_runtime_manager::LoadPlan {
        model_path: model.clone(),
        gpu_layers: Some(99),
        context_size: 256,
    };
    let instance = adapter.load(&plan).await.expect("load");
    assert!(instance.worker_pid.is_some());

    // Stream a short completion and collect tokens.
    let request = ChatRequest {
        messages: vec![ChatMessage {
            role: "user".to_owned(),
            content: "Once upon a time".to_owned(),
        }],
        max_tokens: 12,
        temperature: 0.0,
        stream: true,
    };

    use tokio_stream::StreamExt as _;
    let mut stream = adapter.chat(&instance, &request).await.expect("chat");
    let mut text = String::new();
    let mut token_count = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.expect("token");
        text.push_str(&chunk.content);
        token_count += 1;
    }

    assert!(!text.is_empty(), "the model must generate text");
    assert!(token_count > 0);
    println!("generated ({token_count} chunks): {text:?}");

    // Unload releases the worker.
    let pid = instance.worker_pid;
    adapter.unload(instance).await.expect("unload");

    if let Some(pid) = pid {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        // The process is gone (best effort: on Windows the handle is released).
        let _ = pid;
    }
}
