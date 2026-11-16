> **English** | [中文](ADR-012-core-owns-presentation.zh.md)

# ADR-012 — The core owns presentation, the shells only render

**Status:** Accepted (implementation decision for Task 3)

## Context

Spec §4 requires the three native UIs to share one information architecture and
the same terms. Three independent implementations of "how do we describe a
degraded core" will drift: one says *degraded*, another *partially available*,
and a third shows a green dot because it only looked at an HTTP status.

The alternative design — each shell calls `HealthService` and formats what it
gets — fails in a second way: it puts policy in the UI. Deciding that
`dropped > 0` deserves a warning, or that a planned capability should be greyed
rather than hidden, is a product decision that must be made once.

## Decision

The core exposes `UiService/GetSnapshot` (`proto/ui.proto`) returning exactly
what an Overview and Settings page need:

- tiles with **pre-formatted** values, a severity and an optional detail line;
- capability rows with a status word, a severity and the reason a capability is
  unavailable;
- settings rows with `editable` (false in this build) and a note explaining why;
- configuration findings, attached to the field they concern.

Shells render this payload and nothing else. They may choose *presentation*
(winUI `InfoBar`, GTK row, SwiftUI list) but not *meaning*.

## Consequences

- Renaming a label or changing a threshold is a one-line change in Rust, tested
  without a UI toolkit.
- The payload is testable end to end: `gguf overview` prints it, and the Windows
  shell's unit tests assert the mapping. Three unverifiable shells are avoided.
- The core carries a small amount of text it would not otherwise need. That is
  the point: it is text with product meaning.
- A shell cannot show a capability the core did not advertise, which is how spec
  rule 4 stays true across three codebases.
- Localising the UI means localising the core's strings. Accepted for now; a
  locale field can be added to `UiSnapshotRequest` when it matters.

## Alternatives rejected

| Alternative | Why not |
| --- | --- |
| Each shell reads `HealthReport` and formats it | policy leaks into three codebases; drift is guaranteed |
| Shells share a C# / Rust / Swift library | three toolchains, one FFI surface, and still no single source of truth for text |
| A local HTTP+HTML UI | spec §1.2 explicitly forbids a browser UI as the main interface |
