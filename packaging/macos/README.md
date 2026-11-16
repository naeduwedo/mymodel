> **English** | [中文](README.zh.md)

# macOS packaging (DMG) - **recipe only, not built**

Status: **unverified**. No macOS machine was available. The macOS shell itself
is still planned (see `platforms/macos/README.md`), so a DMG would have nothing
to put in it yet.

## What the DMG must contain

    GGUF AI Engine.app/
      Contents/
        MacOS/GGUF AI Engine          SwiftUI shell binary
        Resources/gguf-core           core daemon (helper)
        Resources/gguf                CLI (also symlinked into /usr/local/bin by the installer)
        Resources/gguf-worker         runtime worker
        Info.plist
        _CodeSignature/               after signing

## Requirements before it can be built

1. The SwiftUI shell must exist and build (`platforms/macos`).
2. A Developer ID certificate for signing, plus notarisation via `notarytool`.
3. A hardened-runtime entitlement for launching child processes (the core is a
   helper, so `com.apple.security.cs.allow-unsigned-executable-memory` is **not**
   needed, but `com.apple.security.cs.disable-library-validation` is, because
   llama.cpp ships its own dylibs).

## Sketch of the build (not executed)

```bash
swift build -c release --package-path platforms/macos
cargo build --release -p gguf-engine-daemon -p gguf-engine-cli -p gguf-engine-runtime-worker
# assemble the .app bundle, copy helpers into Resources/
codesign --force --deep --options runtime --sign "Developer ID Application: ..." "GGUF AI Engine.app"
hdiutil create -volname "GGUF AI Engine" -srcfolder dist/dmg -ov -format UDZO dist/GGUF-AI-Engine.dmg
xcrun notarytool submit dist/GGUF-AI-Engine.dmg --keychain-profile gguf --wait
xcrun stapler staple dist/GGUF-AI-Engine.dmg
```

## Data directory

`~/Library/Application Support/GGUF AI Engine/` (spec §15), created on first run.
Removing the .app must not touch it.
