> [English](native-shells.md) | **中文**

# 原生 UI 外壳（Task 3）

> 规范来源：spec §2.1（平台 UI）、§4（信息架构）。

## 决定这里一切的那条规则

spec §4 要求三个外壳共享同一套信息架构和同一套术语。只有让**核心决定某个值的含义**、
外壳只负责渲染，这一点才可能做到。因此 Task 3 在写任何 UI 代码之前，先加入了展示契约：

```text
proto/ui.proto → UiService/GetSnapshot → UiSnapshot
   overwiew.tiles[]      id, label, value (pre-formatted), severity, detail
   overview.features[]   id, label, status_text, reason, severity
   overview.recent_events[]
   settings.groups[]     title → rows(key, label, value, editable, note, severity)
   settings.issues[]     field, severity, message
```

代码中可见的后果：

- 没有任何外壳负责格式化字节、运行时长或百分比；
- 没有任何外壳决定 “degraded” 或 “planned” 长什么样；
- 尚未构建的能力会带着**原因**以 `planned` 到达全部三个外壳，因为它只有一个来源；
- `gguf overview` 在终端里渲染同一份快照，这正是契约在没有显示器的环境下被测试的方式。

## 三个外壳的状态

| 平台 | 技术栈 | 状态 |
| --- | --- | --- |
| Windows | WinUI 3 + Windows App SDK，C#/XAML | 已实现，可构建，20 个测试通过，其中包含一次真实的端到端测试 |
| Linux | GTK 4 + gtk4-rs | 已按真实 API 编写，**尚未编译**（开发机上没有 GTK）；由 CI 任务验证 |
| macOS | SwiftUI + AppKit | **计划中**——尚未交付代码，见 `platforms/macos/README.md` |

Windows 之所以领先：只有在它的平台上，这项工作才能真正被编译、对着运行中的核心启动并测试。写三个
无法验证的外壳然后把 Task 3 称为完成，会违反 spec 规则 4。

## Windows 外壳结构

```text
GgufAiEngine.Shell.Core     net8.0, no WinUI
   Endpoint.cs              pipe name from the data root (shared FNV-1a vectors)
   CoreClient.cs            gRPC over NamedPipeClientStream; TryConnectAsync → null
   SnapshotMapper.cs        UiSnapshot → view models; no policy

GgufAiEngine.Shell          net8.0-windows, WinUI 3
   app.xaml(.cs)            application entry point
   MainWindow.xaml(.cs)     Overview + Settings, polling, stale-state handling
   Generated/*.cs           C# stubs, generated from proto/ and committed

GgufAiEngine.Shell.Core.Tests
   20 xunit tests: endpoint vectors, mapping, end-to-end against gguf-core
```

### 为什么生成的 C# 要提交进仓库

WinUI 项目在 `CoreCompile` 之前会运行 `XamlPreCompile`——一次真正的 C# 预处理——而且它
很早就读取自己的文件列表。因此由构建目标生成的文件来得太晚，`Grpc.Tools` 自身挂到编译
链上的钩子也撑不过这个顺序。把桩代码提交进仓库就消除了这个顺序依赖；
`scripts/generate-csharp-protos.ps1 -Check`（在 CI 中运行）会在 `Generated/` 与 `proto/`
不一致时让构建失败。

### 失败行为

| 情况 | 用户看到什么 |
| --- | --- |
| 没有核心在运行 | 错误横幅 “Core not connected — start it with gguf-core”，最后一次快照被标记为过期 |
| 核心重启 | 下一次轮询成功后横幅消失；通道被重建 |
| 核心有应答但缺少某项能力 | 能力行显示 `planned` 并附上核心给出的原因；Overview 保留 “not implemented” 磁贴 |
| 配置非法 | 核心拒绝启动；外壳的横幅显示连接失败，`gguf doctor` 显示字段级原因 |

## 外壳绝不能做的事

1. 编造一个核心没有发送的数字。
2. 显示一个什么都不做的控件——因此本次构建中每个设置行都是 `editable = false` 并附一条
   说明。
3. 打开网络端口。传输方式是管道/socket，外壳绝不回退到 TCP。
4. 记录或显示 token（spec §9.3）。
