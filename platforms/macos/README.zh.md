> [English](README.md) | **中文**

# macOS 外壳（SwiftUI + AppKit）—— **未实现**

状态：**计划中**。这里有意还没有任何 Swift 源码。

## 为什么这个目录里没有代码

写一个无法验证的外壳，会比一个都不写更糟：

* 开发机上没有 Swift 工具链，所以这里的任何东西都无法编译 —— 仓库会带着一个看起来已完成、却从未构建过的文件；
* 一个正确的客户端需要由 `protoc-gen-swift` + `protoc-gen-grpc-swift` 从 `proto/*.proto` 生成 Swift protobuf 存根；没有它们，任何 Swift 文件要么编译不过，要么就得伪造线格式；
* spec 规则 4 禁止交付看起来受支持、实际并非如此的东西。

Windows 外壳已实现，并针对真实的核心守护进程做过测试；Linux 外壳是对着真实的 gtk4-rs API 写的，但同样未验证。macOS 是唯一两者都没有的平台，因此被标为计划中，并记录在 `docs/architecture/task-status.md`。

## 实现它需要什么

1. 一台装有 Xcode 的 macOS 机器或 CI 运行器（GitHub Actions 的 `macos-latest`）。
2. `PATH` 上有 `protoc-gen-swift` 和 `protoc-gen-grpc-swift`，并把它们接进一个 `scripts/generate-swift-protos.sh`，该脚本照搬 `scripts/generate-csharp-protos.ps1`（生成到 `Sources/GgufShell/Generated`，提交，让 CI 在出现漂移时失败）。
3. 一个 `Package.swift`，依赖 `grpc-swift` 和生成出来的 target。
4. `CoreBridge`：通过位于 `~/Library/Application Support/GGUF AI Engine/run/core.sock` 的 Unix socket 连接，并调用 `UiService/GetSnapshot` —— 与 Windows 外壳所发的是同一个调用。
5. 为 Overview 和 Settings 编写 SwiftUI 视图，精确渲染快照的各个字段：磁贴、功能行（带状态文本和原因）、设置分组和问题项。Swift 里不做任何格式化或严重级别逻辑。
6. 照搬 `GgufAiEngine.Shell.Core.Tests` 的测试：用共享的 FNV-1a 向量验证端点推导、快照映射，以及一个针对 socket 上真实 `gguf-core` 二进制文件的端到端测试。

## 已为它定下的设计决策

- **统一内存不是 VRAM。** macOS 报告的是内存拓扑，GPU 页面不得声称一个独立显存的数字（spec §5.2）。
- **Metal 是唯一的后端。** CUDA/ROCm 条目在这个平台上显示为不受支持，而不是被隐藏。
- **端点按数据根划分**，与 Windows 同一条规则：`run/core.sock` 位于数据根内部，因此两个安装永远不会互相撞车。
