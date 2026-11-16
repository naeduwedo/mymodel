> [English](ADR-013-cli-based-hardware-probes.md) | **中文**

# ADR-013 — 通过厂商 CLI 探测 GPU，而非厂商 FFI

**状态：** 已接受（Task 4 的实现决策）

## 背景

Task 4 需要真实的数字：显存、空闲显存、计算能力、温度、功耗、PCIe 链路和互连拓扑。
权威来源是厂商库——NVML、`amdsmi`、Metal。绑定它们意味着 C FFI（`unsafe`）、一份驱动
版本矩阵、按操作系统区分的动态加载，以及一类本代码库其余部分根本不可能出现的崩溃。

工作区禁止 `unsafe`，只有两个经过评审的 Windows 管道模块例外（ADR-010）。加入 NVML
绑定，要么意味着大幅放宽该例外，要么意味着引入一个把同样的 FFI 藏在背后的依赖 crate。

## 决策

通过随驱动一同发布的厂商 CLI 来探测：

| 厂商 | 工具 | 说明 |
| --- | --- | --- |
| NVIDIA | `nvidia-smi --query-gpu=... --format=csv` + `topo -m` | 稳定、有文档、机器可读 |
| AMD | `rocm-smi --json` | 按文档化的 schema 解析；**尚未在真实硬件上验证**，因此报告为 `Experimental` |
| Apple | `system_profiler SPDisplaysDataType -json` | 操作系统自身报告的内容 |

该选择带来的后果，明确列出：

- 每次探测都是**工具输出的纯函数**——`crate::runner::CommandRunner` 是接缝，因此测试回放
  录制的输出，而不需要 GPU；
- 工具缺失时结果为 `Unknown`（“我们无法询问”），而工具回答“没有设备”时结果为
  `Unsupported`（“我们问过了；答案是没有”）；
- 代价是每次清单运行多启动一个进程，通过在 `HardwareService` 中加入 2 秒缓存来缓解；
- 工具输出在到达 UI 或日志之前会清除 ANSI 转义序列，因为即使被管道重定向，
  `nvidia-smi` 仍会给表头加上装饰。

## 已知限制（已接受）

- **轮询粒度。** 通过 CLI 做逐样本遥测比 NVML 慢。如果 Task 7 需要亚秒级监控，可以在某个
  能力标志后面加入一条可选的 NVML 路径，并为它单独出一份 ADR——它不应取代 CLI 路径，
  因为那条路径让这个 crate 无需硬件也可测试。
- **`nvidia-smi topo -m` 在某些 Windows 驱动版本上会失败。** 这正是为什么拓扑被报告为
  `Unknown` *并附上驱动消息*，而不是“没有 P2P”。在开发机上的一次真实运行复现了这一点。
- **AMD 解析尚未验证。** 在探测结果中标记为 `Experimental`，因此它不会被误当成已验证的
  测量值（规格 §14.2）。

## 验证

- 35 个单元测试，包括解析在真实 2×NVIDIA 机器上录制的输出而得到的 NVIDIA CSV，以及一个
  明确的测试：拓扑矩阵失败时会产生带驱动原话的 `Unknown`。
- 端到端测试运行真实的守护进程并断言结构性诚实：空的 GPU 列表必须附带原因，P2P 永远不会
  被标为“已测试”，且没有任何转义序列到达客户端。
