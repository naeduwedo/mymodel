> [English](ADR-016-llama-cpp-http-server.md) | **中文**

# ADR-016 - 通过 HTTP 服务器集成 llama.cpp，而非 FFI

**状态：** 已接受（Task 6 的实现决策）

## 背景

规格 8.3 要求一个 llama.cpp 适配器：加载 GGUF、运行 chat、流式输出 token、卸载到 GPU、
从工作进程崩溃中恢复。存在两条路径：链接 libllama（不安全的 C FFI、构建/版本矩阵、
崩溃会拖垮核心），或者通过 HTTP 驱动官方的 llama-server 二进制。

## 决策

把 llama-server 作为独立的工作进程来驱动，并通过 HTTP 与它通信：

    core spawns llama-server (own address space)
    core talks HTTP to /health and /completion (SSE streaming)

后果：

- 适配器拥有工作进程的生命周期：启动、就绪轮询、捕获 stderr、先优雅杀死再强制杀死；
- 模型或驱动的崩溃会变成一个附带 stderr 的 WorkerExited 错误，而核心继续存活（ADR-007）；
- 能力是探测出来的，不是假定的：后端来自随附的 DLL，CUDA 设备数量来自 --list-devices；
- 内存由运行时管理；适配器报告 engine_managed_paging 为不支持，并给出该原因（ADR-005）；
- GPU 卸载通过 --n-gpu-layers 参数完成。

## 已知限制（已接受）

- 提示词是由对话历史拼接而来的纯文本；带模型模板的、功能更丰富的 /chat/completions
  端点属于后续工作。
- Token 流是解码后的 JSON，而不是原始管道；对本地工作进程来说够用。
- 每个工作进程一个模型，与 llama-server 的设计一致。

## 验证

- 提示词拼接与后端检测的单元测试。
- 选择性端到端测试：本机上真实的 llama-server（RTX 4090 + RTX 3060，CUDA）加载一个
  1.1 MiB 的 llama GGUF，流式输出 12 个 token，然后卸载。
