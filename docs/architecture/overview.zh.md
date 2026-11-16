> [English](overview.md) | **中文**

# 架构概览

> 规范来源：[`GGUF-AI-Engine-Engineering-Spec.md`](../../GGUF-AI-Engine-Engineering-Spec.md) §3。
> 本文档说明本仓库中的代码如何实现它，并记录哪些内容是被刻意**尚未**构建的。
>
> 自 Task 2 起，IPC 传输、gRPC 服务和 SQLite 层均已实现；相关内容见
> [`ipc-and-storage.md`](ipc-and-storage.zh.md)。

## 进程模型（spec §3.1）

```text
Native Desktop UI (WinUI 3 / GTK4 / SwiftUI)
        │  gRPC over named pipe (Windows) / Unix socket (Linux, macOS)
        ▼
Rust Core Daemon  — gguf-core
   ├── config        typed schema, defaults, validation, path containment
   ├── event-bus     bounded, typed, replayable events to every UI
   ├── engine-core   lifecycle state machine, health, single-instance lock
   ├── database      SQLite + versioned migrations            (Task 2)
   ├── api-server    REST + OpenAI-compatible gateway         (Task 2)
   ├── hardware      CPU/RAM/storage/GPU probes               (Task 4)
   ├── gguf          bounds-checked GGUF parser               (Task 5)
   ├── model-hub     Hugging Face search/download             (Task 5)
   ├── gpu-manager   topology, budgets, leases                (Task 7)
   ├── memory-manager VRAM/RAM/NVMe orchestration             (Task 7)
   └── runtime-manager / runtime-llama-cpp                    (Task 6)
        │
        └── Runtime Worker processes — gguf-worker (ADR-007)
```

核心拥有**策略与资源台账**；工作进程拥有**推理**。除非用户明确要求
（`[core] keep_workers_after_ui_exit`），否则 UI 崩溃绝不能终止模型工作进程。

## 分层

| 层 | crate | 规则 |
| --- | --- | --- |
| 表现层 | `platforms/`（计划中） | 无业务逻辑；只与核心通信。 |
| 传输 | `crates/protocol`、`crates/api-server` | 版本化，能力协商。 |
| 策略 | `crates/engine-core`、`gpu-manager`、`memory-manager` | 每一份预算和每一个租约的唯一所有者。 |
| 领域 | `crates/gguf`、`model-hub`、`hardware` | 纯函数、可测试、无 I/O 策略。 |
| 基础设施 | `crates/database`、`telemetry`、`config` | 可替换，不做领域决策。 |

依赖方向严格向下。`engine-core` 只依赖 `config` 和 `event-bus`；没有任何东西依赖
`apps/`。

## Task 1 实际实现了什么

用 `cargo run -p gguf-engine-cli -- doctor` 可以验证任何一项声明。

### 已实现并测试

- **配置**（`crates/config`）：来自 spec §11 的完整 `config.toml` schema，严格解析
  （拒绝未知键），逐字段范围校验与跨字段校验，以及路径包含检查——配置的目录若解析后
  位于数据根之外，会被拒绝而不是悄悄使用。
- **路径**（`crates/config::paths`）：平台数据根目录（spec §15）、词法规范化、幂等的
  目录创建。
- **事件总线**（`crates/event-bus`）：带序列号、UTC 时间戳和关联 id 的有界广播，为
  重新连接的 UI 保留有界重放环，并用显式的背压/滞后计数代替无界内存。
- **生命周期**（`crates/engine-core::lifecycle`）：spec §3.3 中的状态机，状态迁移经过
  校验；每一次迁移都会发布到总线。
- **健康**（`crates/engine-core::health`）：逐组件报告、机器可读的快照，以及
  `is_healthy()`——它拒绝把正在启动的核心判为健康。
- **单实例锁**（`crates/engine-core::lock`）：独占的 OS 文件句柄；进程死亡时由 OS
  自动释放，因此崩溃不会留下失效的锁。
- **CLI**（`apps/cli`）：`doctor`、`config`、`serve`、`version`，处处支持 `--json`，
  退出码有明确含义。
- **守护进程**（`apps/core-daemon`）：配置 → 目录 → 锁 → 生命周期 → 健康检查定时
  循环，Ctrl+C 优雅关闭。
- **遥测**（`crates/telemetry`）：受环境变量过滤的 tracing 输出到 stderr，可选美化或
  JSON 格式；过滤指令非法时会回退并记录日志。
- **CI**：在 Linux、Windows 和 macOS 上运行 fmt、clippy（`-D warnings`）和测试，另加
  MSRV 检查和 `unsafe` 守卫。

### 刻意未实现

| 能力 | 原因 | 出处 |
| --- | --- | --- |
| 基于命名管道 / Unix socket 的 gRPC | **已完成（Task 2）** | `crates/ipc`，`tests/tests/ipc_e2e.rs` 中的 e2e 测试 |
| SQLite schema 与迁移 | **已完成（Task 2）** | `crates/database`，`migrations/0001_init.sql` |
| HTTP / OpenAI 兼容 API | 下一步 | `api-server` crate `STATUS = Planned`；不启动任何监听器 |
| 硬件 / GPU 探测 | **已完成（Task 4）**——见 [hardware-inventory.md](hardware-inventory.zh.md) | `hardware` crate `STATUS = Planned` |
| GGUF 解析器、Hub 下载 | **已完成（Task 5）**——见 [gguf-and-model-hub.md](gguf-and-model-hub.zh.md) | `gguf`、`model-hub` `STATUS = Planned` |
| llama.cpp 适配器 | Task 6 | `runtime-llama-cpp` `STATUS = Planned`；`gguf-worker` 以退出码 4 结束 |
| 非 LLM 运行时（图像、视频、语音） | **已完成**——见 [multimodal-runtimes.md](multimodal-runtimes.zh.md) |
| 多 GPU 规划、分页 | **已完成（Task 7）**——见 [multi-gpu-and-memory.md](multi-gpu-and-memory.zh.md) | `gpu-manager`、`memory-manager` `STATUS = Planned` |
| 原生 UI 外壳 | Task 3 | `platforms/` 中只有 README |

因此守护进程运行在 `degraded` 而不是 `ready`：UI 必须能展示真实情况，健康报告则把
`ipc` 和 `database` 列为不健康组件并给出原因。

## 真实性机制

spec 规则 4（“没有假功能”）是结构性强制执行的，而不是靠自觉：

1. `gguf_engine_protocol::supported_features()` 是 UI 读取的唯一清单，并有测试断言未
   实现的功能不在其中。
2. 计划中的 crate 暴露 `STATUS = Status::Planned`。
3. `gguf doctor` 在通过的检查旁边打印 `planned` 的工作。
4. 守护进程的健康报告把未实现的组件标记为不健康，而不是省略它们。

## 失败语义

| 失败 | 行为 |
| --- | --- |
| 配置非法 | 启动失败，为每一个错误给出 `field: reason`，而不只是第一个。 |
| 路径位于数据根之外 | 被拒绝（`PathEscape`），绝不静默接受。 |
| 第二个核心实例 | 以退出码 3 退出，并在消息中给出持有者的 pid。 |
| 慢速 UI 订阅者 | 事件被丢弃并计数；发布者永不阻塞。 |
| 控制台无法接收日志 | 日志只写到 stderr，因此关闭 stdout 绝不会改变一条成功命令的退出码。 |
