# ADR-016 - llama.cpp integration through its HTTP server, not FFI

**Status:** Accepted (implementation decision for Task 6)

## Context

Spec 8.3 requires a llama.cpp adapter: load a GGUF, run chat, stream tokens,
offload to GPU, recover from worker crashes. Two paths exist: link libllama
(unsafe C FFI, build/version matrix, crashes take down the core) or drive the
official llama-server binary over HTTP.

## Decision

Drive llama-server as a separate worker process and speak HTTP to it:

    core spawns llama-server (own address space)
    core talks HTTP to /health and /completion (SSE streaming)

Consequences:

- the adapter owns the worker lifecycle: spawn, readiness poll, capture stderr,
  graceful kill then hard kill;
- a model or driver crash is a WorkerExited error with stderr attached, and the
  core survives (ADR-007);
- capabilities are probed, not assumed: backends from shipped DLLs, CUDA device
  count from --list-devices;
- memory is runtime-managed; the adapter reports engine_managed_paging as
  unsupported with that reason (ADR-005);
- GPU offload is the --n-gpu-layers argument.

## Known limits (accepted)

- Prompt is plain text folded from chat history; the richer /chat/completions
  endpoint with the model template is a follow-up.
- Token stream is decoded JSON, not a raw pipe; fine for a local worker.
- One model per worker process, matching llama-server design.

## Verification

- Unit tests for prompt folding and backend detection.
- Opt-in end-to-end test: real llama-server on this machine (RTX 4090 + RTX
  3060, CUDA) loads a 1.1 MiB llama GGUF, streams 12 tokens, unloads.
