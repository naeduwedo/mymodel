> [English](README.md) | **中文**

# platforms/

原生 UI 外壳（Task 3）。每个目录放一个平台的实现；目前一个都还没开始，任何外壳在真正能连上核心守护进程之前，都不会在产品里被列为可用。

| 目录 | 技术栈 | 备注 |
| --- | --- | --- |
| `windows/` | WinUI 3、Windows App SDK、C#/XAML | 需要 Windows App SDK 工作负载和 .NET；无法只靠 Rust 工作区构建。 |
| `linux/` | GTK4 + Libadwaita、gtk4-rs | 需要 GTK 4 和 libadwaita 的开发头文件；落地后作为 Cargo 工作区成员接入。 |
| `macos/` | SwiftUI + AppKit | 需要 Xcode；独立的包，不属于 `cargo build`。 |

适用于这三者的 UI 规则：不含业务逻辑、不直接访问文件或 GPU，并且每一处能力门禁都来自核心守护进程的功能列表。
