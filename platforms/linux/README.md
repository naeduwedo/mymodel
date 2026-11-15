# Linux shell (GTK4 + Libadwaita) — source only in this checkout

This is a real GTK4/gtk4-rs shell, not a placeholder: it builds an
`ApplicationWindow` with a `NavigationView`-equivalent sidebar (Overview /
Settings) and renders the same `UiSnapshot` the Windows shell renders, using
the same `gguf_engine_ipc::CoreClient`.

It is **not** part of the root Cargo workspace, and it is **not** built on
Windows or macOS, because gtk4-rs needs GTK 4 development headers:

```bash
# Debian/Ubuntu
sudo apt install libgtk-4-dev libadwaita-1-dev pkg-config
# Fedora
sudo dnf install gtk4-devel libadwaita-devel pkg-config
# Arch
sudo pacman -S gtk4 libadwaita
```

Then:

```bash
cd platforms/linux
cargo build
cargo run
```

Honesty note: this shell has **not** been compiled or run by the agent that wrote
it (the development machine is Windows with no GTK 4). It is therefore marked
`unverified` in `docs/architecture/task-status.md` until a Linux CI job builds
it. Do not claim the Linux UI works before that job is green.
