# Implementation status

Tracks the roadmap in spec §17. Updated in the same commit that changes the
status.

Legend: **done** — implemented, tested, and usable. **partial** — scaffolding
exists and is honest about what is missing. **planned** — nothing beyond a
documented crate shell.

| Task | State |
| --- | --- |
| 1 Repository bootstrap | **done** |
| 2 Protocol + core | **done** |
| 3 Native UI shells | **partial** (Windows done, Linux written, macOS planned) |
| 4 Hardware inventory | **done** (NVIDIA verified on real 2-GPU machine) |
| 5 GGUF + model hub | **done** |
| 6 llama.cpp adapter | **done** (real GPU inference verified) |
| 7 Multi-GPU + memory | **done** (benchmark harness pending) |
| 8 Multimodal runtime plugins | **done for image/video/ASR/TTS** (three verified end to end); music has no runtime |
| 9 Packaging | **partial** (Windows MSIX built; Linux/macOS recipes; migration + crash logs done) |

## Task 1 — Repository bootstrap: **done**

| Deliverable | State | Evidence |
| --- | --- | --- |
| Cargo workspace (apps + crates) | done | `Cargo.toml`, `cargo build --workspace` |
| Lint / format / test configuration | done | `[workspace.lints]`, `rustfmt.toml`, CI gate |
| CI pipeline | done | `.github/workflows/ci.yml` (fmt, clippy, tests on 3 OSes, MSRV, unsafe guard) |
| Architecture documentation + ADRs | done | `docs/architecture/`, `docs/adr/` |
| `Cargo.lock` committed | done | workspace root |

## Task 2 — Protocol + core: **done**

| Deliverable | State | Evidence |
| --- | --- | --- |
| Protobuf schema | done | `proto/*.proto` |
| Generated gRPC stubs | done | `crates/protocol/build.rs`, vendored `protoc` |
| Capability negotiation | done | `gguf_engine_protocol::features()`, asserted by tests |
| Core daemon | done | `apps/core-daemon`, reaches `ready` with IPC + database |
| Health check | done | `HealthService` over gRPC; `gguf doctor` uses it |
| Platform IPC | done | named pipe (DACL: user + SYSTEM) / Unix socket (0700 dir) |
| Event system | done | `EventService` streaming with bounded replay and resume-by-seq |
| SQLite + migrations | done | `crates/database`, `migrations/0001_init.sql` (19 tables) |
| Security defaults | done | `crates/security`, ADR-010 |

## Task 3 — Native UI shells: **partial**

| Shell | State | Evidence |
| --- | --- | --- |
| Windows (WinUI 3, C#/XAML) | **implemented and tested** | `dotnet build` clean; 20 xunit tests, including end-to-end against the real `gguf-core` over the real named pipe |
| Linux (GTK4, gtk4-rs) | **built, verified, chat implemented** | `platforms/linux/verify.sh`: real core + Xvfb, window survives 8 s, no critical errors. 6 chat-logic tests |
| macOS (SwiftUI, AppKit) | **planned** | `platforms/macos/README.md` states what is missing and why no code is shipped |

The core produces one presentation snapshot (`proto/ui.proto`, `UiService/
GetSnapshot`) and the shells only render it (ADR-012). `gguf overview` renders
the same snapshot in a terminal, which is how the contract is verified without a
display.

## Task 4 — Hardware inventory: **done**

| Deliverable | State | Evidence |
| --- | --- | --- |
| CPU / RAM / storage | done | `crates/hardware/src/host.rs` (sysinfo) |
| NVIDIA probe | done | `nvidia-smi` CSV + topology; **verified on RTX 4090 + RTX 3060** |
| AMD probe | **experimental** | `rocm-smi --json`; parsed against the documented schema, never run on AMD hardware, and reports itself `Experimental` |
| Apple Metal probe | written, **unverified** | `system_profiler`; unified memory handled (no fake VRAM) |
| GPU topology | done | `nvidia-smi topo -m`; P2P as detected / driver-supported / tested |
| Capability status | done | `ProbeOutcome` on every subsystem; `Unknown` never coerced |
| `HardwareService` + CLI | done | `gguf hardware`, Overview tiles, e2e tests |

On this machine the driver refuses to print a topology matrix, so the product
reports **unknown with the driver's own message** rather than "no P2P" — see
[hardware-inventory.md](hardware-inventory.md).

## Task 5 — GGUF + model hub: **done**

| Deliverable | State | Evidence |
| --- | --- | --- |
| GGUF parser | done | `crates/gguf`; bounded prefix read, no mmap, no unsafe; 35 tests (20 malformed + real file) |
| Hugging Face search | done | `HubClient::search`; verified live |
| Download + resume | done | `Range` resume, atomic rename; verified live |
| SHA-256 verification | done | verify before ready; corrupt hash rejected, nothing left behind |
| Manifest | done | derived from bytes + repository facts, never the file name |
| Compatibility report | done | honest `compatible_unverified` for unloaded models |
| SQLite registry | done | `crates/database/registry.rs`; transactional |
| CLI | done | `gguf inspect`, `gguf models search/list/pull` |

Real run: pulled `tinyllamas/stories260K.gguf`, verified SHA-256, committed
atomically, registered it, and `inspect` reports the real architecture parsed
from the bytes. See [gguf-and-model-hub.md](gguf-and-model-hub.md).

## Task 6 — llama.cpp adapter: **done**

| Deliverable | State | Evidence |
| --- | --- | --- |
| Runtime locate/probe | done | PATH, `GGUF_LLAMA_CPP_DIR`, known dirs; backends from shipped DLLs, CUDA devices from `--list-devices` |
| Model load | done | spawns llama-server, waits for `/health`, captures stderr |
| Chat | done | prompt folded from history, `/completion` |
| Streaming | done | SSE parsed into a token stream |
| CPU/GPU offload | done | `--n-gpu-layers`; CUDA verified on this machine |
| Error recovery | done | worker crash is `WorkerExited` with stderr; the core survives (ADR-007) |
| CLI | done | `gguf runtimes`, `gguf chat` |

```text
$ gguf runtimes
llama.cpp
  backends    : cpu, cuda
  multi-gpu   : true       gpu offload : true
  [unsupported] engine_managed_paging - llama.cpp manages its own memory

$ gguf chat --model tinyllamas/stories260K.gguf --prompt 'Once upon a time'
 Anna, Anna, I'm ready to
```

That is the V0.1 milestone from spec §18: a verified model loaded by a real
runtime, streaming real tokens from real GPU hardware. See
[ADR-016](../adr/ADR-016-llama-cpp-http-server.md).

## Task 7 — Multi-GPU + memory: **done**

| Deliverable | State | Evidence |
| --- | --- | --- |
| GPU budgets per device | done | `GpuBudget::for_device`; unknown VRAM yields zero usable |
| Scheduler / planner | done | `plan()` — pure function, 17 tests on the real 2-GPU config |
| Layer split | done | proposed when one card does not fit but the total does |
| Task parallel | done | per-device budgets are independent; leases never cross devices |
| Tensor parallel | gated | only when the runtime declares it (currently never) |
| Resource leases | done | TTL-based and reapable, so a crash cannot pin a GPU |
| RAM / NVMe tiers | done | `TierBudget`, `Pressure`, `grant_prefetch`; NVMe is a cold tier |
| Monitoring | done | `gguf memory` shows VRAM per card, RAM, NVMe and pressure |
| Benchmark | **not implemented** | see below |

```text
$ gguf plan --weights-gib 20
GPU BUDGETS
  NVIDIA GeForce RTX 4090  usable 19.5 GiB of 22.0 GiB free (reserve 2.0 + margin 0.5)
  NVIDIA GeForce RTX 3060  usable  9.3 GiB of 11.8 GiB free (reserve 2.0 + margin 0.5)
                           total usable 28.8 GiB (never treated as one pool)
strategy : layer_split
  00000000:01:00.0 <- 19.5 GiB
  00000000:07:00.0 <-  2.2 GiB

$ gguf memory
  gpu_vram (RTX 4090)  19.5 GiB usable of 24.0 GiB (memory, reserve 2.5 GiB)
  host_ram             38.7 GiB usable of 63.8 GiB (memory, reserve 8.0 GiB)
  nvme (C:\)          542.4 GiB usable of 930.3 GiB (cold, reserve 10.0 GiB)
pressure : nominal
```

### Deliberately not in this task

| Capability | State |
| --- | --- |
| Performance benchmark (tok/s, VRAM peak per strategy) | not implemented; needs a fixed model + prompt set, and belongs with Task 9 validation |
| P2P bandwidth measurement | still `Unsupported`; `p2p_tested` stays false until measured |
| Engine-managed tensor paging | gated off (ADR-005); llama.cpp manages its own memory |
| Wiring the plan into `gguf chat` | the adapter still takes an explicit `--n-gpu-layers`; using a plan there is follow-up work |

## Task 8 — Multimodal runtime plugins: **partial** (vision done, rest declared)

| Modality | State | Evidence |
| --- | --- | --- |
| Text | **done** | llama.cpp, Task 6 |
| Vision (image in) | **done, verified on real hardware** | `llama-mtmd-cli` + `mmproj`; a 27B model answered a real question about a real image |
| Image generation | unsupported | no diffusion adapter; `gguf modalities` says so |
| Video | unsupported | no video adapter |
| ASR | unsupported | no speech-recognition adapter |
| TTS | unsupported | no speech-synthesis adapter |
| Music | unsupported | no music-generation adapter |

```text
$ gguf modalities
modality           status         runtime                detail
text               supported      llama.cpp              streaming chat through llama-server
vision             supported      llama.cpp mtmd         image in, text out; needs an mmproj companion file
image_generation   unsupported    -                      no diffusion runtime adapter is implemented
video              unsupported    -                      no video runtime adapter is implemented
asr                unsupported    -                      no speech-recognition adapter is implemented
tts                unsupported    -                      no speech-synthesis adapter is implemented
music              unsupported    -                      no music-generation adapter is implemented

2 of 7 modalities are implemented in this build

$ gguf vision --model Qwen3.8-27B.gguf --mmproj mmproj-Qwen3.8-27B-Q8_0.gguf --image test.png
The shape in the image is **yellow**. It's a solid yellow circle set against a blue...
```

Multimodal manifests pair `mmproj-*.gguf` companions with their weights and mark the
projector `required`; an unrelated projector is reported as an orphan rather than
silently attached (ADR-018).

### Deliberately not in this task

| Capability | Why |
| --- | --- |
| Diffusion / video / audio / music adapters | no runtime is installed or specified; marking them supported would be a fake feature |
| A resident vision session | `llama-mtmd-cli` is one-shot, so each request reloads the model; the CLI states this cost |
| Vision over the gRPC API | `InferenceService` is still unimplemented (Task 9 follow-up) |

## Task 9 — Packaging: **partial** (Windows verified, Linux/macOS recipes)

| Deliverable | State | Evidence |
| --- | --- | --- |
| Windows MSIX | **built and verified** | `scripts/package-windows.ps1` → 69.8 MB package; `makeappx unpack` succeeds and lists all binaries |
| Linux deb / rpm | **built, installed, launched** | package includes the GTK4 shell; installing it and running the desktop entry's `Exec=` path works under Xvfb |
| macOS DMG | recipe only | `packaging/macos/README.md`; the shell it would wrap is still planned |
| Signing / verification | documented, not automated | signing is a release decision (ADR-019); scripts emit unsigned artifacts |
| Auto-update | **not implemented** | needs a signed channel + rollback policy; half of it would be worse than none |
| Runtime independent update | design only | runtimes install per-user, outside the package (spec §15) |
| User data migration | **done** | `Database::open_with_backup` backs up before migrating, aborts if the copy fails; 6 tests |
| Crash logs | **done** | panic hook writes `crash-<ms>.log`; `gguf doctor` lists them by path |
| Docs | done | `packaging/README.md` plus one README per platform |

```text
$ ./scripts/package-windows.ps1
makeappx  : ...\10.0.26100.0\x64\makeappx.exe
version   : 0.1.0.0
shell     : published to target/package/stage/shell
package   : target/package/windows/GGUFAIEngine-0.1.0-x64.msix
size      : 69.8 MB
```

```text
$ gguf doctor
[ok  ] database           schema v2 (expected v2), sqlite 3.50.2
[ok  ] crash reports      none in ...\logs
```

### Deliberately not in this task

| Capability | Why |
| --- | --- |
| Linux / macOS installers | no machine to build them on; a recipe that was never built is not a deliverable |
| Code signing | needs a certificate the project does not have; automating it would produce untrustworthy packages |
| Auto-update | requires a signed channel, downgrade policy and rollback; the database already refuses downgrades |
| Runtime auto-install | runtimes are per-user and independent by design (spec §15) |

## Follow-up after Task 9 — chat over gRPC: **done**

Task 6 gave the CLI an in-process chat. The UI shells need the same capability
through the core, which is what `InferenceService` now provides.

| Deliverable | State | Evidence |
| --- | --- | --- |
| `InferenceService.Chat` | done | streams `ChatChunk`; the same path the shells use |
| Model resolution | done | a file path, or a registered repository id resolved to its newest revision's weights |
| Unload on disconnect | done | a `Drop` guard unloads even when the client vanishes mid-answer |
| Concurrency | one model | a second request gets `RESOURCE_EXHAUSTED` instead of an unexplained wait |
| `gguf ask` | done | chat through the running core over gRPC |
| End-to-end test | done, opt-in | `chat_e2e` streams real tokens and rejects an unknown model |

```text
$ gguf ask --model ggml-org/models --prompt 'Once upon a time there was' --max-tokens 20
 Anna was very excited and collecting
[20 chunks, 20 tokens, 32 ms, 625.0 tok/s]

$ gguf ask --model ggml-org/models --prompt 'The sky is' --max-tokens 12 --json
{"answer":" Anna, I'm scar","chunks":12,"elapsed_ms":16,"ok":true,
 "tokens_generated":12,"tokens_per_second":750.0}
```

Two failure modes are handled because they were found by tests, not by reading:

- a client that disconnects mid-stream would otherwise leave a `llama-server`
  holding VRAM until the core restarted, so the unload lives in a `Drop` guard;
- a server-streaming RPC can report its failure as the stream's first item rather
  than at call time, so the test accepts either and still refuses an empty answer.

## Follow-up after Task 9 - OpenAI-compatible gateway: **done**

| Deliverable | State | Evidence |
| --- | --- | --- |
| `GET /health` | done | `{"status":"ok"}` |
| `GET /v1/models` | done | reads the real registry: `ggml-org/models` |
| `POST /v1/chat/completions` | done | non-streaming and SSE streaming; real tokens |
| Security defaults | done | loopback-only by default; non-loopback bind without a token is refused before a socket opens |
| CORS | opt-in only | no headers are sent unless the operator asks |
| Tests | done | 15 tests, several over a real loopback socket |

```text
$ curl 127.0.0.1:18888/v1/models
{"object":"list","data":[{"id":"ggml-org/models","object":"model","owned_by":"llama"}]}

$ curl 127.0.0.1:18888/v1/chat/completions -d '{"model":"ggml-org/models",...}'
{"object":"chat.completion","choices":[{"message":{"role":"assistant",
  "content":" Anna, Anna, I'm "},"finish_reason":"stop"}],
 "usage":{"prompt_tokens":0,"completion_tokens":16,"total_tokens":16}}

unknown model -> 404 {"error":{"code":"model_not_found"}}
```

The gateway is a protocol translator over `InferenceService`, not a second
inference path (ADR-020). `prompt_tokens` is 0 because the runtime does not
count them; a guess would look authoritative while being wrong.

## Follow-up - Windows shell chat: **done**

| Deliverable | State | Evidence |
| --- | --- | --- |
| Chat pane | done | model id, prompt, transcript, Send/Stop |
| Streaming render | done | tokens appear as they arrive; Stop keeps the partial answer |
| Toolkit-free logic | done | `ChatSession` and `ChatTranscript` are unit tested without a window |
| C# tests | done | 33 passing (was 20) |

Verified with UI Automation against a live core: the window exposes the Chat
navigation item and renders real values (`2 device(s), 36.0 GiB VRAM`,
`Intel(R) Core(TM) i9-14900KF`, `24 physical / 32 logical cores`).

`crates/inference` was deleted: it was a placeholder, and the real implementation
lives in `crates/ipc/src/inference.rs`.
## Follow-up - Linux shell verified: **done**

Building the GTK4 shell on Linux for the first time found two defects that the
Windows-only gate could never see:

| Defect | Consequence |
| --- | --- |
| `crates/ipc/src/client.rs` `#[cfg(unix)]` connector captured a borrowed `&str` where `'static` was required | the Unix-socket client **did not compile at all** |
| `glib` was used but never declared in `platforms/linux/Cargo.toml` | `MainContext::spawn_local` did not resolve |

Verification is now executable rather than asserted:

```text
$ bash platforms/linux/verify.sh
== starting the core
   core ready
== launching the shell under Xvfb for 8 seconds
   shell stayed alive and was stopped by the timeout
== errors logged by the shell
   none
== PASS
```

`verify.sh` treats exit 124 (the timeout killing a still-running app) as success
and anything else as failure, so a shell that starts and dies cannot pass.

## Follow-up - embeddings: **done**

| Deliverable | State | Evidence |
| --- | --- | --- |
| `InferenceService.Embeddings` | done | new RPC; loads the worker in embedding mode |
| `POST /v1/embeddings` | done | OpenAI shape; string or array input |
| Honest refusal | done | a model that cannot run in embedding mode returns `FAILED_PRECONDITION`, not a meaningless vector |

```text
$ curl 127.0.0.1:18888/v1/embeddings -d '{"model":"ggml-org/models","input":["hello","world"]}'
object: list  count: 2  vector[0] length: 64
first 3 values: 1.0292757, -1.3270983, 0.1801005
```

llama.cpp exposes embeddings on its native `/embedding` endpoint (one input per
request) and its `/v1/embeddings` answers 400 on this build, so the gateway
translates and sends one request per input rather than pretending to batch.
## Follow-up - Linux packages built: **done**

The recipe stopped being a claim. `packaging/linux/build-packages.sh` produces both
formats and then verifies the .deb's file list against what the README promises.

```text
created package: dist/gguf-ai-engine_0.1.0_amd64.deb      (12.1 MB)
created package: dist/gguf-ai-engine-0.1.0-1.x86_64.rpm  (12.4 MB)
== verifying the .deb file list
   all expected files present
```

Then installed and run, which is the only thing that proves a package works:

```text
$ gguf version --json      -> { "os": "linux", "protocol": 2 }
$ gguf doctor              -> ipc: /tmp/.../run/core.sock, database schema v2, core ready
$ dpkg -r gguf-ai-engine
   user data survived the uninstall
```

The doctor output also proves the **Unix-socket IPC transport** works: the same
code path that did not compile before this round.

A defect found while doing it: `nfpm` expands `{{ .Env.VERSION }}` in the package
metadata but writes the raw template into the file name, producing
`gguf-ai-engine_{{ .Env.VERSION }}_amd64.deb`. The script now resolves the version
itself and refuses to run if any template survives into the generated config.
## Follow-up - Linux chat page: **done**

The Linux shell now has the same chat capability as the Windows one: model id,
prompt, transcript, Send / Stop / Clear.

The rules live in `platforms/linux/src/chat.rs` with no GTK dependency, so they
are tested without a display:

| Rule | Why |
| --- | --- |
| an empty prompt is refused before reaching the core | it would load a model to answer nothing |
| the pending prompt is added to the request but not the transcript | a failed call must not leave history claiming a turn that never happened |
| an empty delta is not a token | the runtime sends them as keep-alives |
| Stop keeps the partial answer | the user watched those tokens arrive |

Aborting the streaming task drops the stream, which is what tells the core to
unload the model - the same unload-on-disconnect rule the gRPC service enforces.

```text
$ cd platforms/linux && cargo test
test result: ok. 6 passed; 0 failed
```
## Follow-up - the Linux package ships the GUI: **done**

The .deb shipped the CLI and the core but not the shell, so the desktop entry
pointed at a path that did not exist. It was documented as "inert until the GTK4
shell is packaged" - which stopped being true once the shell built, and became a
lie the moment someone installed it.

`build-packages.sh` now builds `platforms/linux` too and installs the result at
the exact path the desktop entry names. The package declares `Depends:
libgtk-4-1` and links the system GTK rather than bundling a second copy.

Verified by installing the .deb and launching that path:

```text
-rwxr-xr-x 4194024 /usr/lib/gguf-ai-engine/gguf-ai-engine-ui
Exec=/usr/lib/gguf-ai-engine/gguf-ai-engine-ui
   core ready
   packaged UI stayed alive for 8s
   no critical errors
== PASS
```
## Follow-up - the four modalities: **implemented**

Task 8 asked for image, video, audio and music adapters. Three runtimes were built
from source and driven for real; music has none, and the report says so.

| Runtime | Modality | Verified |
| --- | --- | --- |
| stable-diffusion.cpp (`sd-server`, CUDA 13.4) | image generation | **yes** - 512x512 PNG in 14.2 s |
| stable-diffusion.cpp | video | **yes** - Wan 2.1 T2V 1.3B: 17 frames in 95.7 s through the adapter; refusal path also verified |
| whisper.cpp (`whisper-cli`, CUDA) | ASR | **yes** - round trip through SAPI speech |
| Piper | TTS | **yes** - and its output transcribes back to the same sentence |
| - | music | **no runtime**, and no adapter claims one |

```text
$ gguf modalities
asr                supported    whisper.cpp            provided by whisper.cpp (...whisper-cli.exe)
image_generation   supported    stable-diffusion.cpp   provided by stable-diffusion.cpp (...sd-server.exe)
music              unsupported  -                      no runtime in this build provides music
text               supported    llama.cpp              streaming chat through llama-server
tts                supported    Piper                  provided by Piper (...piper.exe)
video              supported    stable-diffusion.cpp   provided by stable-diffusion.cpp
vision             supported    llama.cpp mtmd         image in, text out

6 of 7 modalities are implemented in this build
```

### The architecture that made this possible

`crates/multimodal` gives every runtime a **manifest** (spec §2.3: OS, archs,
backends, model architectures, quantizations, parallel modes, offload, paging,
modalities), a **probe** that finds its binary and says where it looked, and an
**adapter** that speaks its protocol. The modality registry is then derived from
probes instead of being a hand-written list: a capability is offered because a
runtime was found, and a missing one carries the install hint.

### A measurement that changed a conclusion

The first image took 412.9 s. That is implausible for a 4090, so it was
investigated rather than reported: a stray `llama-server` from an earlier test was
holding 21 GB of the 24 GB card, and killing it brought the same render to
**14.2 s** - a 29x difference. This is exactly the failure the GPU manager's
per-device budgets and expiring leases exist to prevent, seen from the outside.

### Deliberately not done

| Capability | Why |
| --- | --- |
| Music generation | no ggml-based music runtime is integrated; adding one means a new adapter and a manifest, not a flag |
| Rendering with Wan 2.2 A14B | it needs both 13.3 GB transformers plus a 6.3 GB encoder; the 1.3B variant is what fits a 24 GB card |
| Starting the diffusion server automatically | the model must be on its command line; the engine points at a server the user runs |
| `/v1/audio` in the UI shells | the endpoints exist; the shells have no audio page yet |
## Follow-up - shells get the media services: **done (Windows)**

The Create page drives `MediaService` over the same gRPC transport the shells
already use, so no shell grows an HTTP client.

| Piece | State |
| --- | --- |
| `proto/media.proto` + `MediaService` | done: image, video, speech, transcription |
| `crates/multimodal::MediaEngine` | done: one implementation shared by gRPC and the HTTP gateway |
| `crates/ipc/src/media.rs` | done: missing runtime -> `FAILED_PRECONDITION` with the probe's reason |
| Windows Create page | done: prompt, size, Generate, Synthesise, status lines |
| Linux Create page | **not done** |

Verified with UI Automation after selecting the page (a collapsed panel is not in
the automation tree, so the selection is part of the check):

```text
elements after selecting Create: 32
  [ok] Create   [ok] Image    [ok] Speech   [ok] Generate   [ok] Synthesise
  [ok] Describe the image...   [ok] Text to speak...   [ok] Width   [ok] Height
```
## Follow-up - video progress stream: **done**

`GenerateVideo` is now server-streaming (`VideoProgressEvent`), because a clip takes
minutes and a blocking call cannot distinguish working from hung.

| Layer | State |
| --- | --- |
| `VideoProgress` in the adapter | done - stage, detail, completed/total, unit, elapsed, queue position |
| `multimodal::video::generate_with_progress` | done - one implementation, `generate` delegates to it |
| `MediaEngine::video_with_progress` | done |
| gRPC streaming | done - the render runs in its own task and reports over a channel |
| CLI | done - one line per stage change, on stderr |
| Windows shell | done - ProgressBar + status line |
| Linux shell | **not done** |

```text
$ gguf video --prompt 'a fox in a snowy forest' --steps 8
conditioning (2s)
sampling 0/8 steps 0% (8s)
sampling 3/8 steps 38% (64s)
vae_decode (154s)
encoding (165s)
```

Verified end to end through the Windows UI: UI Automation set the prompt, clicked
Generate clip, and observed `sampling 3/8 steps` -> `vae_decode` -> `Wrote 424 KiB
(17 frames)`.

Two real defects found by doing it: the adapter never sent `sample_steps` (so the
runtime used its default 20 and tripled the render time), and the CLI rewrote its
progress line on every poll because the elapsed time always changed.
