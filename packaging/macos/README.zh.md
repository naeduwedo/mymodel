> [English](README.md) | **中文**

# macOS 打包（DMG）—— **仅有配方，未构建**

状态：**未验证**。手边没有可用的 macOS 机器。macOS 外壳本身仍在计划中（见 `platforms/macos/README.md`），所以 DMG 目前还没有东西可放进去。

## DMG 必须包含什么

    GGUF AI Engine.app/
      Contents/
        MacOS/GGUF AI Engine          SwiftUI shell binary
        Resources/gguf-core           core daemon (helper)
        Resources/gguf                CLI (also symlinked into /usr/local/bin by the installer)
        Resources/gguf-worker         runtime worker
        Info.plist
        _CodeSignature/               after signing

## 构建之前的要求

1. SwiftUI 外壳必须存在并且能构建（`platforms/macos`）。
2. 一个用于签名的 Developer ID 证书，以及通过 `notarytool` 完成的公证。
3. 一个用于启动子进程的 hardened runtime 授权（核心守护进程是一个 helper，所以**不**需要 `com.apple.security.cs.allow-unsigned-executable-memory`，但需要 `com.apple.security.cs.disable-library-validation`，因为 llama.cpp 自带 dylib）。

## 构建草案（未执行）

```bash
swift build -c release --package-path platforms/macos
cargo build --release -p gguf-engine-daemon -p gguf-engine-cli -p gguf-engine-runtime-worker
# assemble the .app bundle, copy helpers into Resources/
codesign --force --deep --options runtime --sign "Developer ID Application: ..." "GGUF AI Engine.app"
hdiutil create -volname "GGUF AI Engine" -srcfolder dist/dmg -ov -format UDZO dist/GGUF-AI-Engine.dmg
xcrun notarytool submit dist/GGUF-AI-Engine.dmg --keychain-profile gguf --wait
xcrun stapler staple dist/GGUF-AI-Engine.dmg
```

## 数据目录

`~/Library/Application Support/GGUF AI Engine/`（spec §15），首次运行时创建。删除 .app 不得碰它。
