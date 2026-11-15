# Linux packaging (deb / rpm / AppImage) - **recipe only, not built**

Status: **unverified**. Nothing in this directory has been produced on a Linux
machine, because none was available while writing it. Treat these as reviewed
recipes that CI must build before the Linux package is claimed to work.

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

## What CI must prove

1. `nfpm` builds both formats without error.
2. `dpkg -c` / `rpm -qlp` list exactly the files above.
3. Installing in a clean container and running `gguf doctor` exits 0.
4. Removing the package leaves `~/.local/share/gguf-ai-engine` intact.
