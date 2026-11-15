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
| 8 Multimodal runtime plugins | **partial** (vision verified; others declared unsupported with reasons) |
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
| Linux (GTK4, gtk4-rs) | **written, unverified** | `platforms/linux`; CI job `platform-shells.yml` is what verifies it (no GTK on the dev machine) |
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
| Linux deb / rpm | recipe only | `packaging/linux/nfpm.yaml` + scripts; **not built** (no Linux machine) |
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