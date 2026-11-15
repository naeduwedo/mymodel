# Linux packaging (deb / rpm / AppImage) - **recipe only, not built**

Status: **unverified on the development machine**. Nothing here has been built
locally, because no Linux machine was available while writing it.

[`.github/workflows/package-linux.yml`](../../.github/workflows/package-linux.yml)
now builds and tests these packages on every relevant push. The recipe becomes a
deliverable the first time that job runs green - not before.

## What is here

| File | Purpose |
| --- | --- |
| `nfpm.yaml` | One definition for both .deb and .rpm (keeps them from drifting) |
| `gguf` | `/usr/bin/gguf` wrapper that execs the real binary from /usr/lib |
| `gguf-ai-engine.desktop` | Desktop entry (inert until the GTK4 shell is packaged) |
| `postinstall.sh` | Prints where data will live; creates nothing system-wide |
| `preremove.sh` | On removal, tells the user where their models are instead of deleting them |
| `README.packaged.md` | Installed as the package's documentation |

## Build (on a Linux machine)

```bash
cargo build --release -p gguf-engine-daemon -p gguf-engine-cli -p gguf-engine-runtime-worker
nfpm package --config packaging/linux/nfpm.yaml --packager deb --target dist/
nfpm package --config packaging/linux/nfpm.yaml --packager rpm --target dist/
```

## Deliberate decisions

- **No system service.** The core is a per-user process with a per-user data
  directory; installing a root service would give every local user access to
  every user's models and GPU.
- **Runtimes are not shipped.** llama.cpp binaries carry their own CUDA/ROCm
  dependencies and are updated independently (spec §15). Shipping them in the
  package would freeze the runtime at package-release time.
- **Removal keeps user data.** A 40 GB model re-download is a real cost, and
  removing a package is not a request to delete models.
- **AppImage is not defined yet.** It needs a self-contained bundle including
  GTK4, which depends on the Task 3 Linux shell that is still unverified.

## What the CI job proves

The workflow asserts all four of these, and fails the build if any regresses:

1. `nfpm` builds both formats without error.
2. `dpkg -c` / `rpm -qlp` list the three binaries, the PATH wrapper and the
   desktop entry.
3. The package installs and `gguf version --json` runs.
4. Uninstalling leaves `~/.local/share/gguf-ai-engine` intact - the test writes
   a file there first and checks it afterwards.