> [English](hardware-inventory.md) | **中文**

# 硬件清单（Task 4）

> 规范来源：spec §4.2（Hardware 页面）、§5.2（设备模型）、§5.3（拓扑）。

## 唯一规则

每个值要么是**实测的**，要么是**明确未知且带原因**。没有第三种选择，也不会有任何值被
填成 0 来冒充实测结果。

这一点由类型强制，而不是靠自觉：

```text
ProbeOutcome<T> { status, reason, value, provenance }
  status: Supported | Experimental | Unsupported | Unknown
```

| 状态 | 含义 | 本机示例 |
| --- | --- | --- |
| `Supported` | 已实测且可信 | 来自 `nvidia-smi` 的 GPU 列表 |
| `Experimental` | 已解析，但从未在真实硬件上验证 | AMD `rocm-smi` |
| `Unsupported` | 我们问过了，答案是不支持 | “no supported GPU (CPU-only operation)” |
| `Unknown` | 我们没法问 | `nvidia-smi topo -m` 执行失败 |

`Unknown` 与 `Unsupported` 永远不会被合并。一旦合并，缺少一个工具就会在 bug 报告里
变成“你的硬件坏了”。

## 目录结构

```text
crates/hardware/src/
  types.rs        ProbeOutcome, GpuDevice, GpuTopology, PeerLink, provenance
  runner.rs       CommandRunner trait + SystemRunner (the test seam)
  host.rs         CPU / memory / storage via sysinfo
  gpu_nvidia.rs   nvidia-smi CSV + topo -m
  gpu_amd.rs      rocm-smi --json (Experimental)
  gpu_apple.rs    system_profiler (unified memory handled)
  orchestrate.rs  merge vendors, decide topology, build HostSnapshot
  testing.rs      FakeRunner + recorded output helpers (test only)
```

探测是工具输出的纯函数，因此 35 个单元测试无需 GPU 就能运行。

## 在真实硬件上验证

开发机就是 spec 的首要目标机型：**RTX 4090 24 GB + RTX 3060 12 GB，i9-14900KF（24c/32t），
64 GB 内存**。一次实机运行报告如下：

```text
GPUS   status: supported
  [nvidia] NVIDIA GeForce RTX 4090   24.0 GiB total, 22.5 GiB free   compute 8.9
  [nvidia] NVIDIA GeForce RTX 3060   12.0 GiB total, 11.8 GiB free   compute 8.6
TOPOLOGY   status: unknown
  unavailable     : nvidia-smi topo -m failed: <driver message>
```

### 这里的拓扑为什么是未知的

这个驱动的构建会先打印拓扑表头，然后失败，并让 stderr 保持为空（信息落在 stdout 上）。
这两件事都做了处理：

1. `stderr_line()` 回退到 stdout，因此用户看到的是驱动自己的原话，而不是“没有输出”；
2. 这次失败产生 `Unknown` —— 而不是一个暗示“没有 P2P”的空矩阵。

表头里还满是 ANSI 转义序列（`\x1b[4m`），它们在文本到达 UI 或日志之前就会被剥离。

## P2P 是三态，其中只有一态是承诺

```text
p2p_detected            the driver lists the pair as capable
p2p_supported_by_driver the topology allows it
p2p_tested              we moved bytes and measured them   <- always Unsupported until Task 7
```

Windows 外壳和 CLI 都打印“已检测，未测量”，因此在背后没有基准测试的情况下，产品里永远不会
出现“P2P 可用”这句话。

## 统一内存

Apple Silicon 报告 `total_vram_bytes: None`，且 `MemoryInfo.unified_with_gpu` 为 true。
不存在独立的显存池，因此不得显示独立的预算（spec §5.2）。快照上的 `total_vram_bytes()` 是
对报告独立显存池的各设备求出的**展示用总量**；它永远不会被当作可用容量呈现。

## 开销与缓存

每次清单运行会启动一次厂商工具。`HardwareService` 把结果缓存 2 秒，并在阻塞线程池上运行
探测，因此一个按 2 秒轮询的 UI，每次轮询只花一个进程，而不是每个请求一个进程。探测过程中的
panic 会被捕获并转换成一份不可用的清单、以该 panic 作为原因 —— 驱动辅助程序崩溃绝不能把核心
一起带走。

## 本任务不回答的问题

| 问题 | 在哪里回答 |
| --- | --- |
| 我实际能用多少 VRAM？ | Task 7（`usable = free - reserves - margin`） |
| 哪些 GPU 之间能直接通信，速度多少？ | Task 7（基准测试） |
| 哪个模型能放进哪里？ | Task 7（规划器） |
