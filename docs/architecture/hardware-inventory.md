# Hardware inventory (Task 4)

> Normative source: spec §4.2 (Hardware page), §5.2 (device model), §5.3 (topology).

## The one rule

Every value is either **measured**, or **explicitly unknown with a reason**. There
is no third option, and nothing is zero-filled to look like a measurement.

That is enforced by types, not discipline:

```text
ProbeOutcome<T> { status, reason, value, provenance }
  status: Supported | Experimental | Unsupported | Unknown
```

| Status | Meaning | Example on this machine |
| --- | --- | --- |
| `Supported` | measured and trustworthy | GPU list from `nvidia-smi` |
| `Experimental` | parsed but never validated on real hardware | AMD `rocm-smi` |
| `Unsupported` | we asked; the answer is no | "no supported GPU (CPU-only operation)" |
| `Unknown` | we could not ask | `nvidia-smi topo -m` failed |

`Unknown` and `Unsupported` are never collapsed. Doing so is how a missing tool
turns into "your hardware is broken" in a bug report.

## Layout

```text
crates/hardware/src/
  types.rs        ProbeOutcome, GpuDevice, GpuTopology, PeerLink, provenance
  runner.rs       CommandRunner trait + SystemRunner (the test seam)
  host.rs         CPU / memory / storage via sysinfo
  gpu_nvidia.rs   nvidia-smi CSV + topo -m
  gpu_amd.rs      rocm-smi --json (Experimental)
  gpu_apple.rs    system_profiler (unified memory handled)
  orchestrate.rs  merge vendors, decide topology, build HostSnapshot
  testing.rs      FakeRunner + recorded output helpers (test only)
```

Probes are pure functions of tool output, so 35 unit tests run without a GPU.

## Verified on real hardware

The development machine is the spec's primary target: **RTX 4090 24 GB + RTX 3060
12 GB, i9-14900KF (24c/32t), 64 GB RAM**. A live run reports:

```text
GPUS   status: supported
  [nvidia] NVIDIA GeForce RTX 4090   24.0 GiB total, 22.5 GiB free   compute 8.9
  [nvidia] NVIDIA GeForce RTX 3060   12.0 GiB total, 11.8 GiB free   compute 8.6
TOPOLOGY   status: unknown
  unavailable     : nvidia-smi topo -m failed: <driver message>
```

### Why the topology is unknown here

This driver build prints the topology header, then fails, and leaves stderr empty
(the message is on stdout). Both facts are handled:

1. `stderr_line()` falls back to stdout, so the user sees the driver's own words
   instead of "no output";
2. the failure produces `Unknown` — not an empty matrix implying "no P2P".

The header is also full of ANSI escapes (`\x1b[4m`), which are stripped before
the text reaches a UI or a log.

## P2P is a tri-state, and only one of them is a promise

```text
p2p_detected            the driver lists the pair as capable
p2p_supported_by_driver the topology allows it
p2p_tested              we moved bytes and measured them   <- always Unsupported until Task 7
```

The Windows shell and the CLI both print "detected, not measured", so the phrase
"P2P works" never appears in the product without a benchmark behind it.

## Unified memory

Apple Silicon reports `total_vram_bytes: None`, and `MemoryInfo.unified_with_gpu`
is true. There is no separate pool, so no separate budget may be shown (spec
§5.2). `total_vram_bytes()` on the snapshot is a **display total** across devices
that report a discrete pool; it is never presented as usable capacity.

## Cost and caching

Each inventory run spawns the vendor tools once. `HardwareService` caches the
result for 2 seconds and runs the probe on the blocking pool, so a UI polling at
2 s costs one process per poll, not one per request. Probe panics are caught and
converted into an unavailable inventory with the panic as the reason — a driver
helper crashing must not take the core down.

## What this task does not answer

| Question | Where it is answered |
| --- | --- |
| How much VRAM may I actually use? | Task 7 (`usable = free - reserves - margin`) |
| Which GPUs can talk directly, at what speed? | Task 7 (benchmark) |
| Which model fits where? | Task 7 (planner) |
