> [English](README.md) | **中文**

# GGUF AI Engine

跨平台本地 GGUF 模型管理与推理平台。

- **产品规格书：** [GGUF-AI-Engine-Engineering-Spec.md](GGUF-AI-Engine-Engineering-Spec.md) —— 本项目的规范性文档。
- **实现进度：** [docs/architecture/task-status.md](docs/architecture/task-status.zh.md)
- **架构：** [docs/architecture/overview.md](docs/architecture/overview.zh.md)
- **决策记录：** [docs/adr/](docs/adr/README.zh.md)

## 文档

- **[编译与使用手册](docs/BUILD-AND-USAGE.md)** —— 怎么编译、怎么配置、每个模态怎么用。
- [架构](docs/architecture/overview.zh.md) —— 分层、数据流，以及各领域的说明。
- [决策记录](docs/adr/) —— 那些别扭的地方为什么是现在这个样子。
- [任务进度](docs/architecture/task-status.zh.md) —— 哪些做完了，证据是什么。

## 状态

| 阶段 | 状态 |
| --- | --- |
| Task 1 —— 仓库初始化 | **完成**（工作区、配置、事件总线、生命周期、CLI、守护进程、CI、文档） |
| Task 2 —— 协议与核心 | **完成**（protobuf/gRPC、命名管道与 Unix socket IPC、健康、事件流、SQLite 迁移） |
| Task 3 —— 原生界面外壳 | **部分完成**（Windows 外壳已实现并在 CI 中测试；Linux 外壳可编译并在 Xvfb 下以无头方式运行；macOS 待实现） |
| Task 4 —— 硬件盘点 | **完成**（CPU/内存/存储；NVIDIA 在真实双卡机器上验证，AMD 为实验性，Apple 未验证） |
| Task 5 —— GGUF 与模型中心 | **完成**（防御式解析器、Hub 搜索/下载/校验/断点续传、manifest、登记表） |
| Task 6 —— llama.cpp 适配器 | **完成**（经 llama-server 提供流式对话、视觉与 embeddings，ADR-016） |
| Task 7 —— 多卡与内存 | **完成**（每卡预算与租约、规划器、基准测试；ADR-017） |
| Task 8 —— 多模态运行时 | **完成**（图像、视频、语音识别、语音合成、音乐与音效，基于四个独立 C++ 运行时） |
| Task 9 —— 打包 | **完成**（.deb 与 .rpm 在 CI 中构建并安装，GTK4 外壳包含在内） |

任何能力在背后没有真实实现之前，都不会被报告为「支持」（规格 §18）。
`gguf doctor` 打印的特性列表与界面所用的同一份，每个未实现的能力在代码**和**
界面里都标为 `planned`。

## 环境要求

- Rust **stable ≥ 1.93**（`rust-toolchain.toml` 固定了工具链通道；CI 还会校验
  声明的 MSRV）
- Windows 10/11、Linux 或 macOS
- 不强制要求 GPU、驱动或运行时——没有显卡时相关能力会如实报为不可用

## 快速开始

```bash
cargo build --workspace          # 编译所有 crate
cargo test --workspace           # 单元 + 集成 + 二进制端到端测试
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

### 命令行

```bash
# 在前台运行核心守护进程（Ctrl+C 停止）
cargo run -p gguf-engine-daemon -- --data-dir ./data

# 另开一个终端：诊断，包含对运行中核心的实时探测
cargo run -p gguf-engine-cli -- doctor --data-dir ./data

# 生效的配置、解析后的目录与校验发现
cargo run -p gguf-engine-cli -- config --data-dir ./data
cargo run -p gguf-engine-cli -- config --defaults --json
```

`doctor` 只有在确实出问题时才返回非零退出码。核心没有运行会如实报告这一点，
而不是把它算作失败。

### 直接与核心通信

核心通过本地传输协议说 gRPC（Windows 上是命名管道，其他平台是 Unix socket）。
任何 gRPC 客户端都可以用 [`proto/`](proto/) 里的模式接入：

```bash
# 健康、生命周期状态与能力矩阵
grpcurl -plaintext -unix ./data/run/core.sock localhost:1 gguf.v1.HealthService/GetHealth
```

Rust 客户端是 `gguf_engine_ipc::CoreClient`（`connect`、`health`、`ping`、
`subscribe`）。

两个二进制也都接受 `--data-dir <DIR>`（或 `GGUF_AI_ENGINE_DATA_DIR`），
这样开发期间数据不必落在平台的默认位置。

### 数据目录（规格 §15）

| 平台 | 根目录 |
| --- | --- |
| Windows | `%LOCALAPPDATA%\GGUF-AI-Engine\` |
| Linux | `~/.local/share/gguf-ai-engine/` |
| macOS | `~/Library/Application Support/GGUF AI Engine/` |

`models/`、`cache/`、`cache/downloads/`、`runtimes/`、`logs/`、`database/`
在首次启动时创建。配置里的路径如果解析到根目录之外会被拒绝，因此配置文件
永远无法把写入重定向到应用目录以外的地方。

## 工作区结构

```text
apps/core-daemon        gguf-core      核心守护进程（生命周期、健康、单实例锁）
apps/cli                gguf           命令行客户端
apps/runtime-worker     gguf-worker    隔离的运行时工作进程
crates/config           类型化配置、默认值、校验、路径包含检查
crates/event-bus        有界的类型化事件总线，支持重放
crates/engine-core      生命周期状态机、健康、单实例锁
crates/protocol         protobuf/gRPC 代码生成、值类型、特性协商
crates/database         SQLite 模式、版本化迁移、仓储
crates/security         管道 DACL / socket 权限、常量时间令牌
crates/ipc              本地 gRPC 传输：服务端、客户端、服务
crates/telemetry        tracing 配置（日志格式、环境过滤）
crates/<others>         规划中的模块，各自记录其不变量
proto/                  所有服务的 protobuf 模式（已实现 + 未来）
migrations/             版本化 SQL 迁移
tests/                  工作区级别的集成测试与二进制测试
```

完整的规划结构（proto/、platforms/、packaging/、migrations/）已在仓库中建好，
这样路径就是稳定的；空目录带一个 `README.md`，说明将来放什么、属于哪个任务。

## 不可回退的设计规则

1. 未实现的能力标为 `planned`/`unsupported` —— 绝不是一个看起来能用的按钮。
2. 这里不写推理内核；运行时通过适配器集成（ADR-003）。
3. 每一次 GPU/内存/NVMe 分配都要经过带预留量的预算，并配一份会释放的租约。
4. NVMe 是冷层，永远不等价于显存（ADR-006）；操作系统的交换文件不是模型分页机制。
5. 模型仓库里的任何东西都不会被执行（规格规则 8）。
6. 外部进程都是可观测、可取消、可重启的。
7. IPC 表面仅限本地：带「仅当前用户」DACL 的管道，或 `0700` 目录里的 socket。
   `unsafe` 只允许出现在两个 Windows 模块里
   （[ADR-010](docs/adr/ADR-010-unsafe-windows-pipe-acl.zh.md)）。

## 许可

Apache-2.0 —— 见 [LICENSE](LICENSE)。
