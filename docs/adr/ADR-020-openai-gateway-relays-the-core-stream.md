# ADR-020 - The OpenAI gateway relays the core's stream instead of loading models

**Status:** Accepted (implementation decision for the `http_api` follow-up)

## Context

Spec §9.2 asks for a local REST / OpenAI-compatible API. The obvious
implementation is a second inference path inside the gateway: it already has
the model path, so it could load and stream on its own.

That would mean two places that resolve a model id, two that decide GPU
layers, two that handle worker crashes - and they would drift. The UI and the
CLI already talk to the core; a third path would be the one nobody tests.

## Decision

The gateway is a **protocol translator**, not an inference engine:

    OpenAI JSON -> gRPC ChatRequest -> InferenceService -> token stream -> SSE

Consequences:

- one inference implementation, already exercised by the UI and the CLI;
- the gateway inherits model resolution, unloading and the concurrency rule
  without reimplementing any of it;
- a gateway bug can only be a translation bug, which is the part that is
  cheap to test (15 tests, several over a real loopback socket).

### Streaming is the only mode

Non-streaming responses are produced by draining the same stream. Two code
paths would let `stream: true` and `stream: false` disagree about what the
model said, which is the kind of bug that erodes trust in the whole API.

### Security defaults are enforced, not documented

| Rule | Where it is enforced |
| --- | --- |
| Loopback bind by default | `ApiConfig::default()` |
| Non-loopback bind without a token is refused | `ApiConfig::validate()`, before a socket opens |
| Token compared in constant time | `authorise()` |
| No CORS headers unless opted in | `apply_cors()` returns early |
| A 401 says how to authenticate | `WWW-Authenticate: Bearer` |

A local model endpoint reachable from any web page is a confused-deputy hole,
so the default is silence rather than `Access-Control-Allow-Origin: *`.

### Honest accounting

`prompt_tokens` is reported as 0 because the runtime does not count prompt
tokens. Reporting a guess would make `usage` look authoritative while being
wrong.

## Verification

    $ curl 127.0.0.1:18888/v1/models
    {"object":"list","data":[{"id":"ggml-org/models","object":"model","owned_by":"llama"}]}

    $ curl 127.0.0.1:18888/v1/chat/completions -d '{...}'
    {"object":"chat.completion","choices":[{"message":{"role":"assistant",
      "content":" Anna, Anna, I'm "},"finish_reason":"stop"}],
     "usage":{"completion_tokens":16,"total_tokens":16}}

    unknown model -> 404 {"error":{"code":"model_not_found"}}
