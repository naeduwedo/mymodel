# ADR-013 — Probe GPUs through vendor CLIs, not vendor FFI

**Status:** Accepted (implementation decision for Task 4)

## Context

Task 4 needs real numbers: VRAM, free VRAM, compute capability, temperature,
power, PCIe link and interconnect topology. The authoritative sources are vendor
libraries — NVML, `amdsmi`, Metal. Binding them means C FFI (`unsafe`), a driver
version matrix, per-OS dynamic loading, and a class of crash the rest of this
codebase cannot have.

The workspace forbids `unsafe` except for the two reviewed Windows pipe modules
(ADR-010). Adding NVML bindings would mean either widening that exception
considerably or taking a dependency that hides the same FFI behind a crate.

## Decision

Probe through the vendor CLIs that ship with the driver:

| Vendor | Tool | Notes |
| --- | --- | --- |
| NVIDIA | `nvidia-smi --query-gpu=... --format=csv` + `topo -m` | Stable, documented, machine-readable |
| AMD | `rocm-smi --json` | Parsed from the documented schema; **not yet validated on real hardware**, so reported `Experimental` |
| Apple | `system_profiler SPDisplaysDataType -json` | What the OS itself reports |

Consequences of the choice, made explicit:

- each probe is a **pure function of tool output** — `crate::runner::CommandRunner`
  is the seam, so tests replay recorded output instead of needing a GPU;
- a missing tool is `Unknown` ("we could not ask"), while a tool that answers
  "no devices" is `Unsupported` ("we asked; the answer is no");
- the cost is one process spawn per inventory run, mitigated by a 2-second cache
  in `HardwareService`;
- tool output is cleaned of ANSI escapes before it reaches a UI or a log, because
  `nvidia-smi` decorates its headers even when piped.

## Known limits (accepted)

- **Polling granularity.** Per-sample telemetry through a CLI is slower than NVML.
  If Task 7 needs sub-second monitoring, an optional NVML path can be added
  behind a capability flag with its own ADR — it should not replace the CLI path,
  which keeps the crate testable without hardware.
- **`nvidia-smi topo -m` fails on some Windows driver builds.** That is exactly
  why topology is reported as `Unknown` *with the driver message* rather than
  "no P2P". A real run on the development machine reproduced this.
- **AMD parsing is unvalidated.** Marked `Experimental` in the probe result, so it
  cannot be mistaken for a verified measurement (spec §14.2).

## Verification

- 35 unit tests, including NVIDIA CSV parsed from output recorded on a real
  2×NVIDIA machine, and an explicit test that a failed topology matrix yields
  `Unknown` with the driver words.
- End-to-end test that runs the real daemon and asserts structural honesty:
  an empty GPU list must carry a reason, P2P is never "tested", and no escape
  sequence reaches the client.
