# ADR-019 - Packaging ships binaries and recipes, never user data or runtimes

**Status:** Accepted (implementation decision for Task 9)

## Context

An installer decides more than where files land: it decides what survives an
uninstall, what a downgrade can corrupt, and what a crash report may contain.
Three tempting shortcuts are wrong here:

- bundling llama.cpp and its CUDA libraries into the installer, which freezes
  the runtime at package-release time and multiplies the download by ~200 MB;
- deleting the data directory on uninstall, which throws away models a user
  waited hours to download;
- generating a self-signed certificate in the build script, which produces
  packages that install with a scary warning and cannot be trusted.

## Decision

1. **Packages contain binaries and metadata only.** Models, runtimes and the
   database live in the per-user data directory (spec §15) and are created on
   first run.
2. **Uninstall keeps user data.** The Linux `preremove.sh` prints the path; the
   MSIX never writes outside the app's own data directory in the first place.
3. **Runtimes update independently.** They are installed per-user and are not
   part of any installer.
4. **Signing is a release step, not a build step.** Scripts emit unsigned
   artifacts; the trust decision belongs to whoever ships them.
5. **The package version comes from `Cargo.toml`**, substituted into the MSIX
   manifest at build time, so a version can never disagree with the binaries.

## Safety features that make uninstall/upgrade survivable

| Risk | Mitigation |
| --- | --- |
| Downgrade corrupts the schema | `MigrationError::TooNew` refuses to open a database written by a newer build |
| Upgrade loses data | `Database::open_with_backup` copies the database before migrating, and aborts if the copy fails |
| Client and core disagree | `protocol` version is in `gguf version --json`; a mismatch is refused at handshake |
| A crash leaves no evidence | a panic hook writes `crash-<ms>.log`; `gguf doctor` lists them by path |

## Verified vs. written

| Platform | State |
| --- | --- |
| Windows MSIX | **built**: 69.8 MB package; `makeappx unpack` reports success and lists the three core binaries plus the shell |
| Linux deb/rpm | recipe only; no Linux machine was available, and CI must build it before it is claimed |
| macOS DMG | recipe only; the SwiftUI shell it would contain is still planned |

This split is deliberate. Writing three installers and calling Task 9 done
would have produced two artifacts nobody had ever built.
