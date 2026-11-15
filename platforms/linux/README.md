# Linux shell (GTK4) - built and verified

A real GTK4/gtk4-rs shell, not a placeholder: it builds an
`ApplicationWindow` with a stack switcher (Overview / Settings) and renders the
same `UiSnapshot` the Windows shell renders, through the same
`gguf_engine_ipc` client.

## Status

| Check | State |
| --- | --- |
| Compiles | **yes** - GTK 4.22.4, Ubuntu 26.04, Rust 1.100 |
| Runs headless | **yes** - `verify.sh` under Xvfb, against a real core |
| Overview + Settings pages | working |
| Chat page | **implemented** - model id, prompt, transcript, Send/Stop/Clear |

Verified output:

```text
== starting the core
   core ready
== launching the shell under Xvfb for 8 seconds
   shell stayed alive and was stopped by the timeout
== errors logged by the shell
   none
== PASS
```

## Building

```bash
sudo apt-get install -y libgtk-4-dev build-essential pkg-config xvfb   # Debian/Ubuntu
sudo dnf install -y gtk4-devel gcc pkgconf-pkg-config xorg-x11-server-Xvfb   # Fedora

bash platforms/linux/build.sh        # build
bash platforms/linux/verify.sh       # build + prove it runs
```

`build.sh` checks for GTK 4 before invoking cargo, and puts the target
directory on a Linux filesystem by default: building a Rust tree that lives on
a Windows 9p mount is several times slower.

`verify.sh` is the claim, not the build. It starts a real core, waits for
"core ready", launches the shell under `xvfb-run`, and fails unless the window
survives eight seconds (exit 124 from `timeout`) without logging a critical
error. A GUI that starts and immediately dies exits 0 or 1 and fails the check.

## What building it on Linux actually found

Two real defects that the Windows-only gate could never see:

1. **`crates/ipc/src/client.rs`: the `#[cfg(unix)]` connector did not compile.**
   The `service_fn` closure captured a borrowed `&str` while
   `connect_with_connector` requires `'static`. Fixed by owning the path first.
2. **`glib` was used but never declared.** `gtk4` re-exports glib but does not
   make it nameable, so `MainContext::spawn_local` did not resolve.

The argument for building on the platform you claim to support, in one
sentence: `cargo check` on Windows said both files were fine.

## Chat

The chat page mirrors the Windows one: a model id, a prompt, the transcript, and
Send / Stop / Clear. Two behaviours are deliberate:

- **Stop keeps the partial answer.** The user watched those tokens arrive;
  discarding them would be a surprise. Aborting the task also drops the stream,
  which is what tells the core to unload the model.
- **An empty prompt never reaches the core.** It would otherwise load a model to
  answer nothing.

The rules live in `src/chat.rs`, free of GTK, so they are tested without a
display (`cargo test` in this directory: 6 tests).

## Not implemented

| Capability | Why |
| --- | --- |
| System prompt field | `Transcript::set_system` exists and is tested, but no widget sets it yet |
| Model picker | The model id is typed. A picker needs the registry list, which the Overview page does not request |
## Data root

`$GGUF_AI_ENGINE_DATA_DIR`, else `~/.local/share/gguf-ai-engine` (spec §15).
The shell never invents a path the core would not use.