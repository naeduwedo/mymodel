> **English** | [中文](README.zh.md)

# Packaging (Task 9)

| Platform | Format | Status |
| --- | --- | --- |
| Windows | MSIX | **built and verified** (`scripts/package-windows.ps1`) |
| Linux | deb / rpm via nfpm | **built, installed, launched** - the package includes the GTK4 shell and its desktop entry works |
| macOS | DMG | **recipe only** — the shell itself is still planned |

```text
packaging/
  windows/   Package.appxmanifest (template), README with the verified build
  linux/     nfpm.yaml, wrapper, desktop entry, install/remove scripts
  macos/     README describing the bundle, signing and notarisation requirements
```

## What every package must guarantee

These are the invariants the scripts are written to protect; a packaging change
that breaks one of them is a regression, not a detail.

1. **User data outlives the package.** Removing the app never deletes models or
   the database. Linux `preremove.sh` prints the data path; the MSIX writes only
   inside the per-user data directory.
2. **Runtimes are separate.** llama.cpp binaries are installed per-user and
   updated independently (spec §15). They are not frozen into the installer.
3. **One version source.** The package version is read from `Cargo.toml`; the
   manifest template carries a placeholder that the script substitutes.
4. **Least privilege.** The MSIX requests `runFullTrust` and `internetClient`
   only. There is no broad filesystem capability, because the app never needs it.
5. **Signing is a release decision, not a build step.** Scripts produce unsigned
   artifacts; whoever ships them signs them with a certificate they control.

## Version and update metadata

`gguf version --json` is the machine-readable identity of a build:

```json
{ "cli": "0.1.0", "core": "0.1.0", "protocol": 2, "os": "windows", "arch": "x86_64", "profile": "release" }
```

`protocol` is the wire-protocol version: a client and core that disagree on it
refuse to talk, so an updater can detect an incompatible mix instead of failing
at the first call.

## Crash logs

The core installs a panic hook that writes `crash-<unix-ms>.log` into the
`logs/` directory (`gguf_engine_telemetry::crash`). `gguf doctor` lists any
reports it finds so support can ask for the newest one by path. A crash report
records the version, platform, source location and panic message — never model
contents, paths outside the data directory, or credentials.

## Auto-update

Not implemented. Doing it properly requires a signed update channel, a
downgrade policy and a rollback path for a failed migration; shipping half of
that would be worse than shipping none. The pieces that *are* in place:

- the database refuses to open when written by a newer build, so an accidental
  downgrade cannot corrupt user data;
- migrations back up the database first (`Database::open_with_backup`);
- `protocol` versioning lets a client detect an incompatible core.