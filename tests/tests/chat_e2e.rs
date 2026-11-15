//! End-to-end chat over the real IPC transport (spec §9).
//!
//! These need a running core, an installed llama.cpp and a downloaded model, so
//! they are `#[ignore]`d and opt-in. They never silently skip: without the
//! environment they fail loudly rather than pretending to pass.
//!
//! Run with:
//!
//!     GGUF_CHAT_TESTS=1 cargo test -p gguf-engine-tests --test chat_e2e -- --ignored

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use gguf_engine_protocol::pb;
use tokio_stream::StreamExt;

/// A core running for the duration of a test.
struct Core {
    child: Child,
    data_root: PathBuf,
}

impl Drop for Core {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.data_root);
    }
}

fn require_env() {
    if std::env::var("GGUF_CHAT_TESTS").ok().as_deref() != Some("1") {
        panic!(
            "set GGUF_CHAT_TESTS=1 to run this test; it needs a real llama.cpp and a \
             downloaded model"
        );
    }
}

fn model_id() -> String {
    std::env::var("GGUF_CHAT_MODEL").unwrap_or_else(|_| "ggml-org/models".to_owned())
}

/// Prepare an isolated data root seeded from `GGUF_CHAT_DATA`.
///
/// Two tests running in parallel must not share a data root: the core takes a
/// single-instance lock, so the second one would fail to start and the failure
/// would look like a chat bug. Each test therefore gets its own copy.
fn seeded_data_root(label: &str) -> PathBuf {
    let source = std::env::var("GGUF_CHAT_DATA").expect(
        "set GGUF_CHAT_DATA to a data directory with a model already pulled          (e.g. gguf models pull ... --data-dir <dir>)",
    );
    let source = PathBuf::from(source);
    assert!(
        source.join("database").is_dir() && source.join("models").is_dir(),
        "GGUF_CHAT_DATA must contain database/ and models/: {}",
        source.display()
    );

    let data_root =
        std::env::temp_dir().join(format!("gguf-chat-e2e-{}-{label}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data_root);
    std::fs::create_dir_all(&data_root).unwrap();

    // Copy only what the core needs; the database holds the registry entry that
    // maps the model id to a file.
    for directory in ["database", "models"] {
        copy_tree(&source.join(directory), &data_root.join(directory));
    }

    data_root
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn start_core(label: &str) -> Core {
    let data_root = seeded_data_root(label);

    let binary = core_binary();
    let child = Command::new(binary)
        .arg("--data-dir")
        .arg(&data_root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("starting gguf-core");

    Core { child, data_root }
}

/// Connect, retrying until the core is listening.
///
/// The readiness check *is* the real connection on purpose. On Windows the core
/// keeps exactly one listening pipe instance, so a separate "is it up?" probe
/// consumes that instance and the connection that follows races with the server
/// re-creating it - which shows up as a spurious `NotRunning`.
async fn connect_with_retry(data_root: &Path) -> gguf_engine_ipc::client::CoreClient {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match gguf_engine_ipc::client::connect(data_root).await {
            Ok(client) => return client,
            Err(err) if Instant::now() < deadline => {
                let _ = err;
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            Err(err) => panic!("the core did not accept a connection within 20s: {err}"),
        }
    }
}

fn core_binary() -> PathBuf {
    if let Ok(path) = std::env::var("GGUF_CORE_BIN") {
        return PathBuf::from(path);
    }
    let mut path = std::env::current_exe().unwrap();
    // target/<profile>/deps/<test>.exe -> target/<profile>/gguf-core.exe
    path.pop();
    path.pop();
    path.push(if cfg!(windows) {
        "gguf-core.exe"
    } else {
        "gguf-core"
    });
    path
}

#[test]
#[ignore = "needs llama.cpp and a downloaded model; set GGUF_CHAT_TESTS=1"]
fn a_chat_streams_tokens_over_the_local_transport() {
    require_env();

    let core = start_core("streams");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    runtime.block_on(async {
        let client = connect_with_retry(&core.data_root).await;

        let request = pb::ChatRequest {
            model_id: model_id(),
            messages: vec![pb::ChatMessage {
                role: "user".to_owned(),
                content: "Once upon a time".to_owned(),
            }],
            task_id: None,
            max_tokens: Some(16),
            temperature: Some(0.0),
            stream: Some(true),
        };

        let mut stream = client.chat(request).await.expect("chat starts");

        let mut answer = String::new();
        let mut chunks = 0;
        let mut done = false;

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.expect("a chat chunk");
            if !chunk.delta.is_empty() {
                chunks += 1;
                answer.push_str(&chunk.delta);
            }
            if chunk.done {
                done = true;
                let progress = chunk.progress.expect("final progress");
                assert!(progress.tokens_generated > 0, "tokens must be counted");
                break;
            }
        }

        assert!(done, "the stream must end with a done chunk");
        assert!(chunks > 0, "at least one token must arrive");
        assert!(!answer.trim().is_empty(), "the answer must not be empty");
        eprintln!("generated {chunks} chunk(s): {answer:?}");
    });
}

#[test]
#[ignore = "needs llama.cpp and a downloaded model; set GGUF_CHAT_TESTS=1"]
fn an_unknown_model_is_reported_as_not_found() {
    require_env();

    let core = start_core("unknown-model");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    runtime.block_on(async {
        let client = connect_with_retry(&core.data_root).await;

        let request = pb::ChatRequest {
            model_id: "definitely/not-registered".to_owned(),
            messages: vec![pb::ChatMessage {
                role: "user".to_owned(),
                content: "hi".to_owned(),
            }],
            task_id: None,
            max_tokens: Some(1),
            temperature: Some(0.0),
            stream: Some(true),
        };

        // A server-streaming RPC can report the failure either at call time or as
        // the stream's first item, so both paths are checked. What must not
        // happen is a successful-looking stream with no answer.
        let text = match client.chat(request).await {
            Err(status) => status.to_string(),
            Ok(mut stream) => match stream.next().await {
                Some(Err(status)) => status.to_string(),
                Some(Ok(chunk)) => {
                    panic!("an unknown model must not produce output, got chunk: {chunk:?}")
                }
                None => panic!("the stream ended without an error or any output"),
            },
        };

        assert!(
            text.contains("not a registered model") || text.contains("not found"),
            "the error must say the model is unknown: {text}"
        );
    });
}

#[test]
fn the_core_binary_path_resolves_next_to_the_test_binary() {
    // A guard against the e2e helpers silently pointing at nothing: if this
    // breaks, the ignored tests above would fail with a confusing spawn error.
    let path = core_binary();
    assert!(
        path.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("gguf-core"),
        "{}",
        path.display()
    );
    assert!(Path::new(&path).parent().is_some());
}
