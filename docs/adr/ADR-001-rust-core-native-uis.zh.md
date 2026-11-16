> [English](ADR-001-rust-core-native-uis.md) | **中文**

# ADR-001 — Rust 核心加三个原生 UI

**状态：** 已接受（spec §16，ADR-001）

## 上下文

Windows、Linux 和 macOS 用户期望原生的外观、观感、菜单、无障碍支持与平台集成（通知、登录项、GPU 厂商工具）。

## 决策

一个 Rust 核心（守护进程 + 库），加上三个独立的原生 UI：WinUI 3 + C#/XAML、GTK4 + Libadwaita（通过 gtk4-rs）、SwiftUI + AppKit。

## 后果

- 业务逻辑、调度与资源策略只存在一份，且位于 Rust 中。
- UI 层不能拥有推理、调度或文件布局；它通过 IPC 调用核心。
- 三套 UI 代码库必须保持功能对等；协议与能力清单是让它们保持一致的那份契约。
- 调试 UI 缺陷从不需要 Rust 工具链，核心崩溃也从不需要重新构建 UI。
