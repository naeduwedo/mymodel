> [English](README.md) | **中文**

# 打包（Task 9）

| 平台 | 格式 | 状态 |
| --- | --- | --- |
| Windows | MSIX | **已构建并验证**（`scripts/package-windows.ps1`） |
| Linux | 通过 nfpm 生成 deb / rpm | **已构建、已安装、已启动** —— 包中包含 GTK4 外壳，其桌面入口可用 |
| macOS | DMG | **仅有配方** —— 外壳本身仍在计划中 |

```text
packaging/
  windows/   Package.appxmanifest (template), README with the verified build
  linux/     nfpm.yaml, wrapper, desktop entry, install/remove scripts
  macos/     README describing the bundle, signing and notarisation requirements
```

## 每个包必须保证什么

这些正是脚本要守护的不变量；一处打包改动只要破坏了其中一条，那就是回归，而不是细节。

1. **用户数据比包活得更久。** 卸载应用绝不会删除模型或数据库。Linux 的 `preremove.sh` 会打印数据路径；MSIX 只写入每用户数据目录内部。
2. **运行时是分开的。** llama.cpp 二进制文件按用户安装，并独立更新（spec §15）。它们不会被冻结进安装程序。
3. **单一版本来源。** 包版本从 `Cargo.toml` 读取；manifest 模板带有一个占位符，由脚本替换。
4. **最小权限。** MSIX 只申请 `runFullTrust` 和 `internetClient`。这里没有宽泛的文件系统能力，因为应用从来不需要它。
5. **签名是发布决策，不是构建步骤。** 脚本产出未签名的产物；由发布它们的人用一个自己掌控的证书来签名。

## 版本与更新元数据

`gguf version --json` 是一次构建的机器可读身份：

```json
{ "cli": "0.1.0", "core": "0.1.0", "protocol": 2, "os": "windows", "arch": "x86_64", "profile": "release" }
```

`protocol` 是线协议版本：客户端与核心守护进程如果在这一项上不一致，就会拒绝通信，因此更新器可以检测出不兼容的组合，而不是等到第一次调用时才失败。

## 崩溃日志

核心守护进程安装了一个 panic 钩子，把 `crash-<unix-ms>.log` 写入 `logs/` 目录（`gguf_engine_telemetry::crash`）。`gguf doctor` 会列出它找到的任何报告，这样支持人员就能按路径索取最新的那一份。崩溃报告记录版本、平台、源码位置和 panic 消息 —— 绝不记录模型内容、数据目录之外的路径或凭据。

## 自动更新

未实现。要把它做对，需要一个签名的更新通道、一份降级策略，以及迁移失败时的回滚路径；只交付其中一半，会比一点都不交付更糟。已经就位的部分是：

- 数据库在被更新的版本写过之后会拒绝打开，因此意外的降级不会损坏用户数据；
- 迁移会先备份数据库（`Database::open_with_backup`）；
- `protocol` 版本化让客户端能够检测出不兼容的核心守护进程。
