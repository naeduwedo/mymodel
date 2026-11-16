> [English](README.md) | **中文**

# 文档

这里的文档大多有两个版本：英文文件（`X.md`）和它的中文译本（`X.zh.md`），
两者保持同步，文件顶部有语言开关。例外是编译与使用手册——它只有中文版，
英文版尚未编写。

## 从这里开始

| 文档 | 回答什么问题 |
| --- | --- |
| [编译与使用手册](BUILD-AND-USAGE.md)（仅中文） | 怎么编译、怎么配置、每个模态怎么用。 |
| [架构总览](architecture/overview.zh.md) | 各部件如何拼在一起。 |
| [任务进度](architecture/task-status.zh.md) | 哪些做完了，证据是什么。 |

## 架构

| 文档 | 范围 |
| --- | --- |
| [overview.zh.md](architecture/overview.zh.md) | 分层、数据流、进程模型 |
| [ipc-and-storage.zh.md](architecture/ipc-and-storage.zh.md) | gRPC 传输、SQLite、迁移 |
| [gguf-and-model-hub.zh.md](architecture/gguf-and-model-hub.zh.md) | GGUF 解析、Hub 下载与校验 |
| [hardware-inventory.zh.md](architecture/hardware-inventory.zh.md) | CPU、内存、存储与 GPU 探测 |
| [multi-gpu-and-memory.zh.md](architecture/multi-gpu-and-memory.zh.md) | 每卡预算、租约、分层 |
| [multimodal-runtimes.zh.md](architecture/multimodal-runtimes.zh.md) | 四个非 LLM 运行时，以及验证了什么 |
| [native-shells.zh.md](architecture/native-shells.zh.md) | Windows、Linux 与 macOS 外壳 |
| [task-status.zh.md](architecture/task-status.zh.md) | 逐任务的完成记录 |

## 决策记录

[adr/README.zh.md](adr/README.zh.md) 索引了 ADR-001 到 ADR-020。每一条记录一个
改起来代价很高的决定，以及背后的理由。

## 规格书

- [GGUF-AI-Engine-Engineering-Spec.md](../GGUF-AI-Engine-Engineering-Spec.md) ——
  规范性产品文档（中文原文，早于这套文档）。