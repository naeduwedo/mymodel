> [English](multi-gpu-and-memory.md) | **中文**

# 多 GPU 调度与内存编排器（Task 7）

> 规范来源：spec §5（多 GPU）与 §6（内存编排器）。

## 塑造整个设计的两条规则

**1. VRAM 按设备分别做预算，绝不相加。**

    usable(device) = observed_free - engine_reserve - safety_margin

`total_usable()` 只为展示而存在，每一处展示它的地方都会说明这一点。除非运行时按层切分，
一个 20 GiB 的张量无法横跨两张卡，因此按合计池做规划，产出的方案会在加载时失败。

**2. 每一次预留都是一份带期限的租约。**

崩溃的工作进程绝不能永久占住一块 GPU。租约带 TTL，可以被回收，所以恢复是自动的，
而不是“重启核心”。

## 规划器是纯函数

    plan(devices, budgets, requirement, policy, runtime_facts) -> Plan | PlanError

它不加锁、不碰 GPU、不启动任何东西，因此可以做到穷尽测试。策略阶梯按顺序如下：

| # | 策略 | 条件 |
| --- | --- | --- |
| 1 | `single_gpu` | 单卡的可用预算就能覆盖整个需求 |
| 2 | `tensor_parallel` | 总量放得下**且**运行时声明支持 TP **且**存在两张同厂商的卡 |
| 3 | `layer_split` | 总量能在一家厂商的卡之间放下（或显式选择跨厂商） |
| 4 | `cpu_offload` | 一张卡加主机内存，前提是运行时能卸载 |
| 5 | error | 什么都放不下，并给出一句话点明具体数字和策略 |

稳定性优先于吞吐：单卡永远是首选，而张量并行在多 GPU 选项里排最后，因为它在实践中最容易失败。

### 策略拦截的是切分，不是加载

当 `allow_multi_gpu = false`，或者卡来自不同厂商且没有 `allow_cross_vendor_model_split` 时，
规划器**不会**立即失败：它会回退到 CPU 卸载，因为模型仍可能在一张卡加内存上跑起来。只有当这个
回退也不可用时，它才返回错误，并且错误里同时点明拦截的策略和缺失的回退。

这一点是一个测试发现的：它期望一个错误，却拿到了一份方案。测试是对的，规划器是错的。

## 内存层级

| 层级 | 内容 | 可寻址？ |
| --- | --- | --- |
| L0 `gpu_vram` | 权重、KV 缓存、激活、热专家 | 是，按设备 |
| L1 `host_ram` | CPU 权重、页缓存、暂存 | 是 |
| L2 `nvme` | 模型文件、冷数据 | **否** —— 一个冷层级（ADR-006） |

`Tier::is_addressable_memory()` 对 NVMe 返回 false，CLI 对它打印 `cold`。没有任何代码路径
把 NVMe 当作内存，操作系统的交换文件也从不被用作模型分页机制。

### 压力与预取

    nominal  -> available > reserve * 1.5
    elevated -> reserve < available <= reserve * 1.5
    critical -> available <= reserve

预取有上限（`max_prefetch_bytes`），并且在压力下**完全停止**：它要和操作系统争抢那部分预留，
所以是第一个被砍掉的。超过剩余余量的请求会被截断，而不是拒绝。

## 验证了什么，以及怎样验证

17 个规划器/租约测试加上 8 个内存测试，使用本机的真实配置（RTX 4090 24 GiB + RTX 3060 12 GiB，
预留 2 GiB + 0.5 GiB 余量）：

| 模型 | 结果 |
| --- | --- |
| 8 GiB | 在 4090 上走 `single_gpu`（可用 19.5 GiB） |
| 20 GiB | `layer_split`，19.5 + 2.2 GiB |
| 60 GiB | `cpu_offload`，并说明会更慢 |
| 60 GiB，无内存卸载 | 报错，指明 VRAM 缺口 |

`gguf plan` 和 `gguf memory` 会在实时硬件上运行这些；两者都展示在
[task-status.md](task-status.zh.md) 中。

## 未实现

| 能力 | 为何缺席 |
| --- | --- |
| 性能基准测试 | 需要固定的模型、prompt 集合和重复次数；应归入 Task 9 的验证，这样数字才可比较 |
| P2P 带宽测量 | `p2p_tested` 保持 `Unsupported`；测量需要基准测试，而不是探测 |
| 引擎托管的张量分页 | 被 ADR-005 关掉：llama.cpp 自己管理内存 |
| 把规划器接进 `gguf chat` | 适配器直接接收 `--n-gpu-layers`；使用方案属于后续工作 |
