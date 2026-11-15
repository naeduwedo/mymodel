# Native UI shells (Task 3)

> Normative source: spec §2.1 (platform UI), §4 (information architecture).

## The rule that shapes everything here

Spec §4 requires the three shells to share one information architecture and one
set of terms. That is only achievable if the **core decides what a value means**
and the shell renders it. So Task 3 added a presentation contract before it added
any UI code:

```text
proto/ui.proto → UiService/GetSnapshot → UiSnapshot
   overwiew.tiles[]      id, label, value (pre-formatted), severity, detail
   overview.features[]   id, label, status_text, reason, severity
   overview.recent_events[]
   settings.groups[]     title → rows(key, label, value, editable, note, severity)
   settings.issues[]     field, severity, message
```

Consequences that are visible in the code:

- no shell formats bytes, uptime or percentages;
- no shell decides what "degraded" or "planned" looks like;
- a capability that is not built arrives as `planned` **with a reason**, in all
  three shells, because there is only one source for it;
- `gguf overview` renders the same snapshot in a terminal, which is how the
  contract is tested without a display.

## Status of the three shells

| Platform | Stack | State |
| --- | --- | --- |
| Windows | WinUI 3 + Windows App SDK, C#/XAML | implemented, builds, 20 tests green including a real end-to-end |
| Linux | GTK 4 + gtk4-rs | written against the real API, **not yet compiled** (no GTK on the dev machine); CI job verifies |
| macOS | SwiftUI + AppKit | **planned** — no code shipped, see `platforms/macos/README.md` |

Why Windows is ahead: it is the platform where this work could actually be
compiled, launched against a live core and tested. Writing three unverifiable
shells and calling Task 3 done would have violated spec rule 4.

## Windows shell structure

```text
GgufAiEngine.Shell.Core     net8.0, no WinUI
   Endpoint.cs              pipe name from the data root (shared FNV-1a vectors)
   CoreClient.cs            gRPC over NamedPipeClientStream; TryConnectAsync → null
   SnapshotMapper.cs        UiSnapshot → view models; no policy

GgufAiEngine.Shell          net8.0-windows, WinUI 3
   app.xaml(.cs)            application entry point
   MainWindow.xaml(.cs)     Overview + Settings, polling, stale-state handling
   Generated/*.cs           C# stubs, generated from proto/ and committed

GgufAiEngine.Shell.Core.Tests
   20 xunit tests: endpoint vectors, mapping, end-to-end against gguf-core
```

### Why the generated C# is committed

A WinUI project runs `XamlPreCompile` — a real C# pre-pass — before
`CoreCompile`, and it reads its file list early. Files produced by a build
target therefore arrive too late, and `Grpc.Tools`' own hook into the compile
chain does not survive that ordering. Committing the stubs removes the ordering
dependency; `scripts/generate-csharp-protos.ps1 -Check` (run in CI) fails the
build if `Generated/` drifts from `proto/`.

### Failure behaviour

| Situation | What the user sees |
| --- | --- |
| No core running | Error banner "Core not connected — start it with gguf-core", last snapshot labelled stale |
| Core restarts | Banner disappears on the next successful poll; the channel is recreated |
| Core answers but a capability is missing | Capability row says `planned` with the core's reason; Overview keeps the "not implemented" tile |
| Configuration invalid | Core refuses to start; the shell's banner shows the connection failure, `gguf doctor` shows the field-level reason |

## What the shells must never do

1. Invent a number the core did not send.
2. Show a control that does nothing — hence `editable = false` plus a note on
   every settings row in this build.
3. Open a network port. The transport is a pipe/socket and the shell never falls
   back to TCP.
4. Log or display a token (spec §9.3).
