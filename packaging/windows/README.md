# Windows packaging (MSIX)

Status: **built and verified on this machine** (Windows 11, Windows SDK
10.0.26100).

## Build

```powershell
cargo build --release -p gguf-engine-daemon -p gguf-engine-cli -p gguf-engine-runtime-worker
./scripts/package-windows.ps1
```

Output: `target/package/windows/GGUFAIEngine-<version>-x64.msix`.

## What the script does

1. reads the version from `Cargo.toml` (one source of truth);
2. stages `core/` (three Rust binaries), `shell/` (WinUI 3, self-contained),
   `runtimes/` (empty: runtimes install per-user at runtime) and `Assets/`;
3. substitutes the version into `Package.appxmanifest` and writes it as
   `AppxManifest.xml` (makeappx rejects the Visual Studio file name);
4. runs `makeappx pack`.

## Verified

```text
makeappx  : ...\10.0.26100.0\x64\makeappx.exe
version   : 0.1.0.0
shell     : published to target/package/stage/shell
package   : target/package/windows/GGUFAIEngine-0.1.0-x64.msix
size      : 69.8 MB
```

Unpacking the produced package lists `Assets/`, `core/` (gguf-core.exe,
gguf.exe, gguf-worker.exe), `shell/`, `AppxManifest.xml` and
`AppxBlockMap.xml`, and `makeappx unpack` reports "Package extraction
succeeded".

## The package is unsigned

Installing it needs Developer Mode, or a signature from a certificate whose
subject matches `Publisher` in the manifest:

```powershell
signtool sign /fd SHA256 /a /f cert.pfx GGUFAIEngine-0.1.0-x64.msix
```

Signing is **not** automated here on purpose: a build script that generates its
own certificate would produce packages nobody should trust, and the trust
decision belongs to whoever ships it.

## Capabilities

Only `runFullTrust` (the shell launches `gguf-core` as a child process) and
`internetClient` (Hugging Face download). No `broadFileSystemAccess`: the app
writes only inside its own data directory.
