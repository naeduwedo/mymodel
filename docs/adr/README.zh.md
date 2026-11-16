> [English](README.md) | **中文**

# 架构决策记录

格式：上下文 → 决策 → 后果。前七条转录自 spec §16（它们是规范性的）；其余是本仓库在实现 Task 1 期间做出的决策，至今仍然具有约束力。

| ADR | 标题 | 状态 |
| --- | --- | --- |
| [001](ADR-001-rust-core-native-uis.zh.md) | 带三个原生 UI 的 Rust 核心 | 已接受（spec） |
| [002](ADR-002-gguf-first.zh.md) | GGUF 优先，而非仅支持 GGUF | 已接受（spec） |
| [003](ADR-003-runtime-adapter.zh.md) | 运行时适配器，不自研内核 | 已接受（spec） |
| [004](ADR-004-capability-driven-multi-gpu.zh.md) | 能力驱动的多 GPU | 已接受（spec） |
| [005](ADR-005-runtime-managed-paging.zh.md) | 运行时管理的分页优先于引擎管理的分页 | 已接受（spec） |
| [006](ADR-006-nvme-is-a-cold-tier.zh.md) | NVMe 是冷层 | 已接受（spec） |
| [007](ADR-007-core-worker-isolation.zh.md) | 核心与推理工作进程是独立进程 | 已接受（spec） |
| [008](ADR-008-bounded-typed-event-bus.zh.md) | 有界、强类型、可重放的事件总线 | 已接受 |
| [009](ADR-009-strict-typed-configuration.zh.md) | 严格、强类型的配置，并约束路径范围 | 已接受 |
| [010](ADR-010-unsafe-windows-pipe-acl.zh.md) | 为 Windows 管道 DACL 开一个狭窄的 `unsafe` 例外 | 已接受 |
| [011](ADR-011-blocking-sqlite-wrapper.zh.md) | 通过阻塞式封装使用 SQLite，而非异步驱动 | 已接受 |
| [012](ADR-012-core-owns-presentation.zh.md) | 核心负责呈现，外壳只负责渲染 | 已接受 |
| [013](ADR-013-cli-based-hardware-probes.zh.md) | 通过厂商 CLI 探测 GPU，而非厂商 FFI | 已接受 |
| [014](ADR-014-defensive-gguf-parsing.zh.md) | GGUF 解析：有界前缀读取，不使用 mmap | 已接受 |
| [015](ADR-015-download-integrity.zh.md) | 下载完整性：就绪前校验，原子提交 | 已接受 |
| [016](ADR-016-llama-cpp-http-server.zh.md) | 通过 llama.cpp 的 HTTP 服务器集成，而非 FFI | 已接受 |
| [017](ADR-017-per-device-budgets-and-leases.zh.md) | 按设备划分的预算与会过期的租约，绝不用求和池 | 已接受 |
| [018](ADR-018-modality-status-not-flags.zh.md) | 模态以状态和理由声明，绝不用标志位 | 已接受 |
| [019](ADR-019-packaging-ships-binaries-only.zh.md) | 打包只分发二进制与配方，绝不分发用户数据或运行时 | 已接受 |
| [020](ADR-020-openai-gateway-relays-the-core-stream.zh.md) | OpenAI 网关转发核心的流，而不加载模型 | 已接受 |

## 编写新的 ADR

创建一个包含 `Status`、`Context`、`Decision`、`Consequences` 各节的 `ADR-NNN-short-title.md`，把它加入上面的表格，并在它所约束的代码中链接到它。ADR 取代另一个 ADR 的方式，是把那个文件的状态改为 `Superseded by ADR-NNN`，而不是改写历史。

## 添加 `unsafe` 代码

工作区禁止 `unsafe_code`。需要它的 crate 必须：(a) 新增一个 ADR，说明为何不存在安全的替代方案；(b) 把例外收窄到仅该 crate；(c) 在 crate 根重新建立 `#![deny(unsafe_code)]`，并按模块单独允许；(d) 更新 CI 的 `deny-unsafe` 任务，使该例外在评审中可见。

当前例外：仅 [ADR-010](ADR-010-unsafe-windows-pipe-acl.zh.md)。
