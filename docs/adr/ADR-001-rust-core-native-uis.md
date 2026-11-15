# ADR-001 — Rust core with three native UIs

**Status:** Accepted (spec §16, ADR-001)

## Context

Windows, Linux and macOS users expect native look, feel, menus, accessibility
and platform integration (notifications, login items, GPU vendor utilities).

## Decision

One Rust core (daemon + libraries) and three independent native UIs:
WinUI 3 + C#/XAML, GTK4 + Libadwaita via gtk4-rs, SwiftUI + AppKit.

## Consequences

- Business logic, scheduling and resource policy exist exactly once, in Rust.
- The UI layer cannot own inference, scheduling or file layout; it calls the
  core through IPC.
- Three UI codebases must be kept in feature parity; the protocol and the
  capability list are the contract that keeps them honest.
- Debugging a UI bug never requires the Rust toolchain, and a core crash never
  requires rebuilding a UI.
