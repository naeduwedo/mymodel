# GGUF AI Engine --- 工程实施规格书

> **文档用途：** 本文件是面向 OpenCode、Codex、Claude Code
> 等代码代理的项目级实施规范（Product + Architecture + Engineering
> Specification）。
>
> **项目代号：** `gguf-ai-engine`
>
> **目标：** 构建一个以 Rust 为核心、支持 Windows / Linux Desktop /
> macOS 原生界面、以 GGUF 模型为优先格式的本地多模态 AI 推理平台。支持
> Hugging Face 模型发现与下载、多 GPU 调度、GPU VRAM / 系统 RAM / NVMe
> 分层资源管理，以及可插拔推理 Runtime。
>
> **重要约束：** 本项目不是 ComfyUI 的替代节点编辑器，也不是重新实现
> CUDA/ROCm/Metal Kernel 的推理框架。必须优先复用成熟 Runtime，并通过
> Adapter 集成。

------------------------------------------------------------------------

## 0. 给代码代理的执行指令

你是本项目的首席软件架构师和实施工程师。必须把本文件视为项目的主规范，并遵循以下规则：

1.  **先检查仓库，再修改代码。** 如果仓库为空，初始化
    Workspace；如果已有代码，先分析并保留可用实现。
2.  **先输出实施计划和风险，再开始大规模修改。**
    但不要只停留在计划；按阶段实际创建文件、实现、测试。
3.  **每个阶段都必须可编译、可测试。**
    不允许一次生成大量无法编译的占位代码。
4.  **不伪造功能。** 未实现的 Runtime、GPU 后端、分页能力必须明确标记为
    `unsupported` 或 `planned`，不能在 UI 上显示为已支持。
5.  **不自行实现推理 Kernel。** 优先集成 llama.cpp 等成熟引擎。
6.  **资源分配必须有预算、预留、Lease、释放和故障处理。**
    不允许多个模块各自无约束地分配 GPU/RAM。
7.  **不把操作系统 Swap/Pagefile 当作正常 NVMe 模型缓存。**
8.  **不执行 Hugging Face 仓库内任意 Python、Shell 或二进制脚本。**
    模型文件、配置和仓库元数据均视为不可信输入。
9.  **所有外部进程必须可监控、可取消、可重启，并记录 stdout/stderr。**
10. **每个 PR/阶段交付：**
    变更摘要、架构说明、运行命令、测试结果、未完成事项和风险。
11. 不要为了追求抽象而过早引入微服务。V0.x 采用模块化单体 Rust Core +
    独立 Runtime Worker。
12. 对任何需要平台 SDK、驱动或 Runtime 才能验证的功能，提供 capability
    probe 和明确的降级路径。

### 0.1 第一轮工作

第一轮不要直接实现所有多模态 Runtime。先完成：

-   仓库结构与工程规范；
-   Rust Workspace；
-   Protobuf 协议；
-   Core Daemon；
-   CLI；
-   Windows / Linux / macOS UI 壳工程；
-   硬件探测抽象；
-   GGUF 文件识别和元数据解析；
-   SQLite 数据库迁移；
-   llama.cpp Runtime Adapter 的最小可运行版本；
-   自动化测试和 CI。

在此基础上按本文件路线图迭代。

------------------------------------------------------------------------

# 1. 产品定义

## 1.1 产品定位

GGUF AI Engine 是跨平台本地 AI
模型管理和推理桌面应用。用户通过一个原生桌面应用完成：

1.  检查本机 CPU、GPU、RAM、NVMe 和驱动；
2.  搜索 Hugging Face 模型仓库；
3.  筛选 GGUF 文件和量化版本；
4.  下载、校验、导入和管理模型；
5.  分析模型与 Runtime 的兼容性；
6.  自动规划 GPU、RAM、NVMe 资源；
7.  加载模型并运行推理；
8.  监控多 GPU 和内存使用；
9.  通过本地 API 为 OpenCode、Codex、Agent 和其他应用提供推理服务。

## 1.2 核心产品原则

-   **GGUF-first，不是 GGUF-only：** 优先发现和运行
    GGUF；多模态模型可以声明必要的非 GGUF 辅助组件，但必须在 Manifest
    中透明列出。
-   **Runtime-first：** 模型能否推理取决于模型架构、组件和 Runtime
    能力，不以文件扩展名作为支持依据。
-   **Native UI：** Windows、Linux、macOS 使用各自原生 UI 技术，不使用
    Electron、浏览器 UI 或 WebView 作为主界面。
-   **One Core, Three Native UIs：** 三个平台共享 Rust
    Core、数据模型、Protobuf API 和功能语义。
-   **Multi-GPU-aware：** 将多 GPU
    作为核心设计，不只是增加一个设备选择框。
-   **Memory-aware：** 统一预算和监控 GPU VRAM、主机 RAM、NVMe
    文件缓存。
-   **Safe by default：** 默认仅监听
    localhost；不执行模型仓库脚本；不静默耗尽系统内存。
-   **Capability-driven：** 只有经过
    Runtime/硬件探测确认的能力才能启用。

## 1.3 非目标

V1.0 前不做：

-   自研通用 CUDA、ROCm 或 Metal 推理 Kernel；
-   自研完整 LLM Transformer 计算引擎；
-   保证任意 GGUF 都可推理；
-   保证不同厂商 GPU 可组成同一个统一显存池；
-   把 NVMe 当作与 VRAM 等价的随机访问内存；
-   以浏览器 UI 取代原生桌面 UI；
-   自动执行仓库内的自定义代码；
-   默认将多个 GPU 的显存容量简单相加后声称可用。

------------------------------------------------------------------------

# 2. 平台与技术栈

## 2.1 平台 UI

  ----------------------------------------------------------------------------------------
  平台              UI 技术           语言              说明
  ----------------- ----------------- ----------------- ----------------------------------
  Windows 10/11     WinUI 3 + Windows C# / XAML         Windows 原生 Fluent UI
                    App SDK                             

  Linux Desktop     GTK4 + Libadwaita Rust / gtk4-rs    GNOME
                                                        原生控件；其他桌面环境尽可能兼容

  macOS             SwiftUI + AppKit  Swift             macOS 原生控件、菜单栏、通知
  ----------------------------------------------------------------------------------------

UI 不是推理逻辑的所有者。UI 只通过统一 IPC/API 调用 Rust Core。

## 2.2 Rust Core

建议：

-   Rust stable；
-   Tokio；
-   tonic + prost（gRPC / Protobuf）；
-   Axum（本地 REST / OpenAI 兼容 API）；
-   Serde；
-   SQLx + SQLite；
-   tracing；
-   reqwest；
-   memmap2；
-   uuid；
-   thiserror / anyhow；
-   parking_lot；
-   tokio-util；
-   sysinfo（通用系统信息，平台专用信息使用 Adapter）。

依赖版本必须在实施时选取当前稳定、互相兼容的版本，并提交
`Cargo.lock`。不要在文档中硬编码未经验证的版本号。

## 2.3 推理 Runtime

第一优先级：

-   llama.cpp：GGUF LLM，及其实际支持的视觉/多模态模型能力。

后续通过 Adapter 接入：

-   GGUF Diffusion Runtime；
-   图片生成 Runtime；
-   视频生成 Runtime；
-   ASR / TTS / Audio / Music Runtime。

每个 Runtime 必须有独立 manifest，声明支持的 OS、CPU 架构、GPU
backend、模型架构、量化、并行模式、Offload、分页和多模态能力。

## 2.4 GPU 后端

目标后端：

-   NVIDIA CUDA；
-   AMD ROCm；
-   Apple Metal；
-   CPU fallback。

后续可评估 Vulkan、DirectML 等，但不得在未验证前列为正式支持。

**平台限制：**

-   CUDA 与 ROCm 依赖对应平台驱动和 Runtime 构建；
-   Apple Silicon 使用 Metal 和统一内存模型，不应套用传统独立 VRAM
    的假设；
-   跨厂商 GPU 默认不进行同一模型的跨设备切分；
-   同厂商多 GPU 是否支持 P2P / Tensor Parallel 必须实测并由 Runtime
    声明。

------------------------------------------------------------------------

# 3. 系统架构

## 3.1 进程模型

采用模块化单体 Core + 隔离 Worker：

``` text
Native Desktop UI
   |  gRPC / IPC
   v
Rust Core Daemon
   |-- Model Hub
   |-- GGUF Registry / Parser
   |-- Hardware Inventory
   |-- GPU Scheduler
   |-- Memory Orchestrator
   |-- Runtime Manager
   |-- Inference Scheduler
   |-- Local API Gateway
   |-- SQLite / Telemetry
   |
   +-- Runtime Worker: llama.cpp
   +-- Runtime Worker: Image
   +-- Runtime Worker: Video
   +-- Runtime Worker: Audio
```

Core 负责策略和资源账本。Runtime Worker 负责实际推理。UI
崩溃不能导致模型 Worker 自动退出，除非用户明确选择随 UI 退出。

## 3.2 通信

-   Windows 本地：Named Pipes；
-   Linux：Unix Domain Socket；
-   macOS：Unix Domain Socket；需要深度系统集成时可增加 XPC Bridge；
-   远程 Worker：gRPC over TLS；
-   Core 内部：Tokio channels / typed events；
-   推理流：gRPC streaming 或 SSE；
-   本地 OpenAI API：HTTP。

所有 IPC 方法必须经过身份/权限检查。Socket/pipe
应设置合适的本机访问权限。

## 3.3 Core 生命周期

状态：

``` text
Stopped -> Starting -> Ready -> Degraded -> ShuttingDown -> Stopped
```

要求：

-   单实例锁；
-   健康检查；
-   UI 断线重连；
-   Worker 心跳；
-   Worker 崩溃恢复；
-   关闭前保存任务状态；
-   升级前停止兼容性受影响的 Worker；
-   防止同一模型被重复加载，除非明确创建独立实例。

------------------------------------------------------------------------

# 4. 原生 UI 信息架构

三套 UI 保持相同的信息架构和术语，遵循平台交互规范。

主导航：

1.  Overview
2.  Hardware
3.  Models
4.  Memory Manager
5.  Multi-GPU
6.  Inference
7.  Tasks
8.  Performance
9.  Runtimes
10. Settings

## 4.1 Overview

展示：

-   Core 状态；
-   CPU / RAM；
-   GPU 列表和 VRAM；
-   NVMe 空间和 IO；
-   已加载模型；
-   当前任务；
-   下载进度；
-   Runtime 状态；
-   最近事件。

## 4.2 Hardware

展示 CPU、RAM、GPU、存储设备、驱动版本、后端可用性、PCIe 拓扑、NUMA、P2P
测试结果。

## 4.3 Models

支持：

-   Hugging Face 搜索；
-   仓库详情；
-   Revision 选择；
-   GGUF 文件筛选；
-   量化筛选；
-   组件依赖；
-   兼容性报告；
-   下载、暂停、恢复、校验；
-   本地导入、移动、删除；
-   模型文件去重。

## 4.4 Memory Manager

按模型实例显示：

-   每张 GPU 的权重占用；
-   每张 GPU 的 KV Cache；
-   激活/工作区预算；
-   RAM 权重和页缓存；
-   staging/pinned buffers；
-   NVMe 模型文件；
-   预读队列；
-   缓存命中率；
-   内存压力；
-   估算与实测差异。

## 4.5 Multi-GPU

展示：

-   GPU 拓扑；
-   设备间链路；
-   P2P 能力和实测带宽；
-   每卡可用显存；
-   当前分配；
-   模型切分方案；
-   任务绑定；
-   设备隔离和并发任务。

## 4.6 Inference

按 Runtime capability 动态显示 Chat、Vision、Image、Video、Audio、Music
等工作区。未支持的能力必须显示为不可用及原因，不得显示假功能。

## 4.7 Tasks

任务状态：

`queued / preparing / downloading / loading / running / postprocessing / completed / failed / cancelled`

支持查看日志、取消、重试和结果位置。

## 4.8 Performance

记录首 token 延迟、prefill、decode tok/s、GPU 利用率、VRAM、RAM、NVMe
吞吐、任务吞吐和生成耗时。

## 4.9 Runtimes

显示 Runtime 版本、安装状态、后端、模型架构支持、能力矩阵、日志和更新。

## 4.10 Settings

模型目录、缓存目录、GPU 策略、内存阈值、API
安全、更新、启动行为、日志保留。

------------------------------------------------------------------------

# 5. 多 GPU 设计

## 5.1 必须支持的执行模式

``` rust
pub enum ExecutionStrategy {
    SingleGpu,
    LayerSplit,
    TensorParallel,
    TaskParallel,
    CpuOffload,
    HybridOffload,
}
```

含义：

-   `SingleGpu`：一个模型绑定一张 GPU；
-   `LayerSplit`：不同 GPU 承担不同模型层；
-   `TensorParallel`：多个 GPU 协同计算同一层，必须由 Runtime
    和模型支持；
-   `TaskParallel`：不同 GPU 运行不同模型或不同任务；
-   `CpuOffload`：部分权重由 CPU/RAM 承担；
-   `HybridOffload`：GPU + RAM + Runtime 支持的 NVMe 按需访问。

## 5.2 设备模型

``` rust
pub enum GpuVendor {
    Nvidia,
    Amd,
    Apple,
    Intel,
    Unknown,
}

pub enum ComputeBackend {
    Cuda,
    Rocm,
    Metal,
    Vulkan,
    DirectML,
    Cpu,
    Unknown,
}

pub struct GpuDevice {
    pub id: String,
    pub name: String,
    pub vendor: GpuVendor,
    pub backend: ComputeBackend,
    pub total_vram_bytes: Option<u64>,
    pub free_vram_bytes: Option<u64>,
    pub memory_bandwidth_bytes_sec: Option<u64>,
    pub compute_capability: Option<String>,
    pub pci_bus_id: Option<String>,
    pub pcie_generation: Option<u32>,
    pub pcie_lanes: Option<u32>,
    pub numa_node: Option<u32>,
    pub temperature_c: Option<f32>,
    pub utilization_percent: Option<f32>,
    pub power_watts: Option<f32>,
    pub peer_access: Vec<String>,
}
```

注意 Apple unified memory 不应伪造为独立 VRAM。设备字段允许
`None`，并通过 memory topology 描述统一内存。

## 5.3 GPU 拓扑

建立 `GpuTopology`，记录：

-   设备；
-   PCIe / NVLink / 其他互连；
-   P2P 可访问性；
-   P2P 实测带宽；
-   NUMA 关系；
-   设备到 Host RAM 的路径；
-   检测来源和可信度。

P2P 必须区分
`detected`、`supported_by_driver`、`tested_successfully`。不能仅凭设备列表就宣称
P2P 可用。

## 5.4 资源预算

每张 GPU 单独计算：

``` text
usable_vram =
    observed_free_vram
  - engine_reserve
  - system_reserve
  - safety_margin
```

模型预算至少包含：

``` text
weights
+ kv_cache
+ activations
+ compute_workspace
+ runtime_overhead
+ fragmentation_margin
```

不能将多个 GPU 的显存简单求和后当作任意 Runtime 都可使用的统一内存池。

## 5.5 调度策略

规划流程：

1.  解析模型架构、权重、量化和组件；
2.  检查 Runtime capability；
3.  发现 GPU 和拓扑；
4.  估算 weights / KV / activations / workspace；
5.  检查每卡可用资源；
6.  生成候选方案：单卡、Layer Split、Tensor Parallel、CPU Offload；
7.  根据 Runtime 能力、通信代价、负载、带宽和稳定性评分；
8.  预留资源 Lease；
9.  启动 Worker；
10. 验证实际分配；
11. 更新监控；
12. 失败时安全释放并尝试允许的降级方案。

默认优先稳定性。Tensor Parallel 只有在 Runtime
明确支持且拓扑适合时才进入候选方案。

## 5.6 异构 GPU 策略

-   NVIDIA + NVIDIA：可尝试 Layer Split；Tensor Parallel 需 Runtime
    支持和基准验证；
-   AMD + AMD：按 ROCm Runtime 能力；
-   Apple：Metal / unified memory 专用策略；
-   NVIDIA + AMD：默认 Task Parallel，不默认同模型切分；
-   GPU + CPU：允许 Runtime 支持的 CPU Offload；
-   多 GPU 并行任务：通过资源 Lease 防止互相抢占。

------------------------------------------------------------------------

# 6. Memory Orchestrator：GPU / RAM / NVMe

## 6.1 分层模型

-   L0 GPU VRAM：计算中权重、KV Cache、激活、热点专家；
-   L1 Host RAM：CPU 权重、文件页缓存、预读和 staging；
-   L2 NVMe：模型文件和冷数据；
-   L3 Hugging Face：远程仓库。

NVMe 是冷数据源，不是等价 VRAM。操作系统 Swap/Pagefile
不是本项目的正常模型分页机制。

## 6.2 两种模式

### Runtime-managed

优先使用 Runtime 原生能力，例如 mmap、GPU layer offload、CPU
offload、专家缓存。此时 AI Engine
负责预算、配置、监控和生命周期，不得重复管理 Runtime
内部的同一块权重内存。

### Engine-managed Tensor Paging

仅对提供 Tensor 生命周期、设备传输和执行同步接口的 Runtime
启用。需要支持：

-   Tensor 元数据索引；
-   生命周期引用；
-   GPU/RAM/NVMe 位置；
-   异步读取；
-   staging buffer；
-   prefetch；
-   eviction；
-   kernel completion fence；
-   错误恢复。

如果 Runtime 不暴露安全的 Tensor 生命周期接口，不允许 AI Engine
强行搬走正在使用的权重。

## 6.3 Tensor Page Metadata

``` rust
pub struct TensorPage {
    pub model_id: String,
    pub tensor_name: String,
    pub file_id: String,
    pub file_offset: u64,
    pub byte_len: u64,
    pub quant_type: Option<String>,
    pub layer_index: Option<u32>,
    pub expert_index: Option<u32>,
    pub placement: TensorPlacement,
    pub pin_count: u32,
    pub last_access_epoch: u64,
    pub access_count: u64,
}
```

`TensorPlacement` 至少表达
`Nvme / HostRam / Gpu(device_id) / RuntimeManaged / Unknown`。

## 6.4 缓存策略

不要只用 LRU。综合：

-   Layer-aware；
-   Expert-aware；
-   Cost-aware；
-   Memory-pressure-aware；
-   Adaptive Prefetch；
-   Runtime execution schedule。

预读必须有上限、取消机制和准确率统计。系统内存压力高时自动降低预读。

## 6.5 内存保护

-   每卡独立显存安全余量；
-   系统 RAM 保留量；
-   NVMe 可用空间保留量；
-   并发加载限制；
-   预读上限；
-   任务优先级；
-   OOM 事件捕获；
-   资源 Lease 超时回收；
-   不在 GPU 工作未完成时释放 Tensor；
-   内存规划失败时拒绝加载并给出原因。

------------------------------------------------------------------------

# 7. GGUF 与 Hugging Face 模型中心

## 7.1 GGUF Parser

实现只读解析器，支持：

-   GGUF magic/version；
-   metadata key/value；
-   tensor descriptors；
-   tensor offsets；
-   alignment；
-   quantization type；
-   tokenizer metadata；
-   architecture metadata；
-   文件边界和异常校验。

必须防御畸形文件：整数溢出、越界偏移、异常长度、超大分配、重复字段和不合法字符串。解析文件时不得直接按文件内声明的长度无上限分配内存。

第一阶段优先使用成熟且许可兼容的解析实现；如果自研
parser，必须用官方/社区 GGUF fixture 和 fuzz testing 验证。

## 7.2 Model Manifest

每个模型仓库版本建立 Manifest，至少包括：

``` json
{
  "schema_version": 1,
  "model_id": "publisher/model",
  "revision": "immutable-commit-or-resolved-revision",
  "architecture": "detected_architecture",
  "modalities": ["text"],
  "format": "gguf",
  "files": [
    {
      "path": "model-Q4_K_M.gguf",
      "role": "weights",
      "required": true,
      "size_bytes": 0,
      "sha256": null
    }
  ],
  "runtime_requirements": [],
  "components": [],
  "license": null
}
```

文件大小、哈希、Revision
等必须来自仓库元数据或下载校验结果，不能由模型名称猜测。

## 7.3 下载功能

必须支持：

-   Hugging Face Hub API；
-   公共和私有仓库；
-   Access Token 安全存储；
-   Revision 固定；
-   文件过滤；
-   Range / 断点续传；
-   并发下载限制；
-   Xet 支持（如所选客户端/协议支持）；
-   ETag / SHA-256 校验；
-   下载暂停/恢复/取消；
-   磁盘空间预检查；
-   文件原子落盘；
-   失败重试和退避；
-   同文件去重。

下载完成后先校验，再登记模型。未校验的文件不得标记为 ready。

## 7.4 模型兼容性报告

每个模型的状态：

-   `supported`：已验证 Runtime 和硬件组合；
-   `compatible_unverified`：结构可识别，但尚未实测；
-   `missing_components`：缺少必要组件；
-   `unsupported_architecture`：Runtime 不支持架构；
-   `unsupported_backend`：当前硬件后端不支持；
-   `insufficient_resources`：预算不足；
-   `unknown`：无法判定。

------------------------------------------------------------------------

# 8. Runtime Adapter 设计

## 8.1 统一能力描述

``` rust
pub struct RuntimeCapabilities {
    pub modalities: Vec<Modality>,
    pub architectures: Vec<String>,
    pub backends: Vec<ComputeBackend>,
    pub supports_multi_gpu: bool,
    pub supports_layer_split: bool,
    pub supports_tensor_parallel: bool,
    pub supports_cpu_offload: bool,
    pub supports_mmap: bool,
    pub supports_lazy_loading: bool,
    pub supports_engine_managed_paging: bool,
    pub supports_streaming: bool,
}
```

能力字段必须能表达"不知道/未测试"，不要把未知一律当作 false 或
true。正式实现可使用
`CapabilityStatus { Supported, Unsupported, Experimental, Unknown }`。

## 8.2 Runtime Trait

``` rust
#[async_trait::async_trait]
pub trait RuntimeAdapter: Send + Sync {
    fn id(&self) -> &'static str;

    async fn probe(
        &self,
        model: &ModelManifest,
        host: &HostSnapshot,
    ) -> anyhow::Result<CompatibilityReport>;

    async fn capabilities(
        &self,
    ) -> anyhow::Result<RuntimeCapabilities>;

    async fn estimate_memory(
        &self,
        model: &ModelManifest,
        request: &InferenceConfig,
        devices: &[GpuDevice],
    ) -> anyhow::Result<MemoryEstimate>;

    async fn load(
        &self,
        model: &ModelManifest,
        plan: &ExecutionPlan,
    ) -> anyhow::Result<RuntimeInstance>;

    async fn infer(
        &self,
        instance: &RuntimeInstance,
        request: InferenceRequest,
    ) -> anyhow::Result<InferenceStream>;

    async fn unload(
        &self,
        instance: RuntimeInstance,
    ) -> anyhow::Result<()>;
}
```

真实接口可根据流式返回、取消和异步任务进行调整，但必须保留 capability
probing、memory estimation、load、infer、unload 的职责边界。

## 8.3 第一批 Runtime

### V0.x

-   llama.cpp：GGUF LLM；
-   仅接入其实际支持且已验证的视觉/多模态能力。

### V0.5 / V1.0

-   Image Runtime；
-   Video Runtime；
-   Audio / ASR / TTS Runtime；
-   Music Runtime。

每个 Adapter 单独版本化、测试和发布。不能因为模型仓库中存在 GGUF
文件就推断对应 Runtime 已经支持。

------------------------------------------------------------------------

# 9. 推理任务与 API

## 9.1 任务模型

``` rust
pub enum TaskStatus {
    Queued,
    Preparing,
    Downloading,
    Loading,
    Running,
    Postprocessing,
    Completed,
    Failed,
    Cancelled,
}
```

任务记录至少包含：

-   task_id；
-   model_id；
-   runtime_id；
-   modality；
-   request parameters；
-   resource plan；
-   GPU allocation；
-   start/end time；
-   status；
-   progress；
-   output references；
-   error code/message。

## 9.2 API

硬件：

``` http
GET /api/v1/hardware
GET /api/v1/gpus
GET /api/v1/gpus/topology
GET /api/v1/memory
GET /api/v1/storage
```

模型：

``` http
GET    /api/v1/models
GET    /api/v1/models/search?q=
GET    /api/v1/models/{id}
POST   /api/v1/models/download
POST   /api/v1/models/import
POST   /api/v1/models/{id}/load
POST   /api/v1/models/{id}/unload
DELETE /api/v1/models/{id}
```

GPU：

``` http
GET  /api/v1/gpu/allocations
GET  /api/v1/gpu/topology
POST /api/v1/gpu/policy
POST /api/v1/gpu/benchmark
POST /api/v1/gpu/rebalance
```

Memory：

``` http
GET  /api/v1/memory/status
GET  /api/v1/memory/allocations
GET  /api/v1/memory/paging
POST /api/v1/memory/policy
POST /api/v1/memory/rebalance
```

推理：

``` http
POST /api/v1/inference/chat
POST /api/v1/inference/vision
POST /api/v1/inference/image
POST /api/v1/inference/video
POST /api/v1/inference/audio
POST /api/v1/inference/music
GET  /api/v1/tasks/{id}
POST /api/v1/tasks/{id}/cancel
GET  /api/v1/tasks/{id}/events
```

OpenAI 兼容：

``` http
GET  /v1/models
POST /v1/chat/completions
POST /v1/embeddings
```

仅在对应模型/Runtime 支持时暴露相关 endpoint。流式响应支持 SSE。

## 9.3 安全默认值

-   API 默认绑定 `127.0.0.1`；
-   局域网监听必须显式开启；
-   局域网模式必须配置认证 Token；
-   CORS 默认拒绝非必要来源；
-   Hugging Face Token 使用系统凭据存储；
-   不把 Token 写入日志；
-   文件路径必须规范化并限制在用户配置目录；
-   防止路径穿越和符号链接攻击；
-   下载文件先写临时文件再原子重命名；
-   模型仓库内容不作为可执行代码；
-   Runtime 二进制需校验来源、版本和哈希。

------------------------------------------------------------------------

# 10. 数据库

第一版 SQLite + SQLx。数据库迁移必须版本化。

核心表：

-   `models`
-   `model_revisions`
-   `model_files`
-   `model_components`
-   `model_manifests`
-   `runtimes`
-   `runtime_capabilities`
-   `devices`
-   `gpu_topology`
-   `gpu_allocations`
-   `model_instances`
-   `memory_allocations`
-   `tensor_pages`
-   `inference_tasks`
-   `inference_sessions`
-   `download_tasks`
-   `performance_metrics`
-   `system_events`
-   `settings`

所有容量使用整数 byte 存储，不用浮点 GB 作为持久化值。时间统一
UTC。数据库迁移要有升级测试和备份策略。

------------------------------------------------------------------------

# 11. 配置文件

示例 `config.toml`：

``` toml
[core]
host = "127.0.0.1"
api_port = 18888
grpc_port = 18889
log_level = "info"
start_with_login = false
keep_workers_after_ui_exit = true

[paths]
models = "./models"
cache = "./cache"
runtimes = "./runtimes"
logs = "./logs"

[downloads]
max_concurrent_downloads = 3
resume = true
verify_hashes = true
temporary_directory = "./cache/downloads"

[memory]
strategy = "auto"
gpu_reserve_bytes = 2147483648
ram_reserve_bytes = 8589934592
nvme_reserve_bytes = 10737418240
max_prefetch_bytes = 2147483648
allow_engine_managed_paging = false

[gpu]
strategy = "auto"
allow_multi_gpu = true
allow_cross_vendor_model_split = false
allow_tensor_parallel = "when_supported"
prefer_task_parallel = true

[runtime]
auto_install = true
isolate_workers = true
auto_update = false

[api]
bind = "127.0.0.1"
authentication = "local_only"
```

所有配置项都必须有 schema、默认值、范围校验、迁移策略和 UI
说明。示例值只是默认起点，必须根据机器动态计算和提示。

------------------------------------------------------------------------

# 12. 推荐 Workspace 目录

``` text
gguf-ai-engine/
├── Cargo.toml
├── Cargo.lock
├── rust-toolchain.toml
├── README.md
├── CONTRIBUTING.md
├── LICENSE
├── .gitignore
│
├── apps/
│   ├── core-daemon/
│   ├── cli/
│   └── runtime-worker/
│
├── crates/
│   ├── engine-core/
│   ├── protocol/
│   ├── config/
│   ├── event-bus/
│   ├── gguf/
│   ├── model-hub/
│   ├── hardware/
│   ├── gpu-manager/
│   ├── memory-manager/
│   ├── runtime-manager/
│   ├── runtime-llama-cpp/
│   ├── inference/
│   ├── api-server/
│   ├── database/
│   ├── security/
│   └── telemetry/
│
├── proto/
│   ├── common.proto
│   ├── hardware.proto
│   ├── gpu.proto
│   ├── memory.proto
│   ├── model.proto
│   ├── runtime.proto
│   ├── inference.proto
│   └── task.proto
│
├── platforms/
│   ├── windows/
│   ├── linux/
│   └── macos/
│
├── runtimes/
│   ├── manifests/
│   └── installers/
│
├── migrations/
├── configs/
├── tests/
│   ├── fixtures/
│   ├── unit/
│   ├── integration/
│   ├── multi-gpu/
│   ├── memory/
│   └── security/
│
├── docs/
│   ├── architecture/
│   ├── adr/
│   ├── api/
│   └── user-guide/
│
├── packaging/
│   ├── windows/
│   ├── linux/
│   └── macos/
│
└── scripts/
```

目录可以在实施中调整，但不得把 UI 业务逻辑混入 Core，也不得让平台 UI
各自实现不同的模型资源调度。

------------------------------------------------------------------------

# 13. 开发阶段与验收标准

## Phase 0 --- 工程骨架

交付：

-   Workspace；
-   Core Daemon；
-   CLI；
-   Protobuf；
-   三个平台 UI 壳；
-   SQLite；
-   tracing；
-   CI；
-   配置系统。

验收：

-   三个平台 UI 均能启动并连接 Core；
-   Core 健康检查可用；
-   数据库迁移成功；
-   基本单元测试通过。

## Phase 1 --- Hardware Inventory + GGUF

交付：

-   CPU/RAM/Storage 检测；
-   GPU Backend Probe 接口；
-   GGUF Parser；
-   Model Registry；
-   Hugging Face 搜索；
-   下载和校验。

验收：

-   能识别有效 GGUF；
-   能拒绝畸形 GGUF；
-   能下载、暂停、恢复和校验文件；
-   UI 显示硬件和模型信息。

## Phase 2 --- llama.cpp Runtime

交付：

-   Runtime 安装/版本管理；
-   Adapter；
-   模型加载/卸载；
-   Chat；
-   SSE streaming；
-   CPU/GPU Offload；
-   运行日志。

验收：

-   能下载一个兼容 GGUF；
-   能自动选择已安装或安装匹配 Runtime；
-   能完成推理；
-   能取消任务；
-   Worker 崩溃后 Core 仍然可用。

## Phase 3 --- Multi-GPU

交付：

-   GPU 拓扑；
-   每卡资源预算；
-   Layer Split；
-   Task Parallel；
-   Runtime 支持时的 Tensor Parallel；
-   P2P 检测；
-   资源 Lease；
-   性能基准。

验收：

-   单卡和双卡均能运行；
-   能验证模型实际使用的 GPU；
-   分配失败可回滚；
-   不支持的策略不会被 UI 伪装为可用；
-   对比单卡/多卡吞吐和显存使用。

## Phase 4 --- RAM / NVMe

交付：

-   mmap；
-   CPU Offload；
-   RAM 缓存；
-   预读；
-   内存压力控制；
-   NVMe IO 监控；
-   Runtime-managed paging；
-   仅在 Runtime 支持时的 Tensor Pager。

验收：

-   能在显存不足时安全使用 CPU/RAM Offload；
-   内存预算可解释；
-   系统 RAM 达到保护阈值时拒绝新加载或降低预读；
-   不依赖 Swap/Pagefile 作为模型分页；
-   所有分页能力有 Runtime capability 证明。

## Phase 5 --- Multimodal

按 Adapter 逐项加入：

-   Vision；
-   Image；
-   Video；
-   Audio / ASR；
-   TTS；
-   Music。

每个 Runtime 都必须有模型
fixture、兼容性测试、资源估算、错误处理和输出验证。无可用 GGUF Runtime
的模型不得标记为支持。

## Phase 6 --- V1.0 Packaging

交付：

-   Windows 安装包；
-   Linux AppImage / deb / rpm；
-   macOS DMG；
-   签名/校验；
-   自动更新；
-   Runtime 独立更新；
-   用户数据迁移；
-   崩溃日志；
-   文档和安装指南。

------------------------------------------------------------------------

# 14. 测试策略

## 14.1 测试层级

-   Unit：GGUF parser、预算算法、配置、Manifest；
-   Integration：Hub 下载、SQLite、Runtime Adapter；
-   Hardware：GPU probe、VRAM、温度、利用率；
-   Multi-GPU：Layer Split、P2P、Task Parallel；
-   Memory：RAM 压力、mmap、预读、取消；
-   Security：畸形 GGUF、路径穿越、下载中断、凭据保护；
-   UI：启动、连接、断线恢复、关键流程；
-   Performance：prefill、decode、吞吐、NVMe IO。

## 14.2 测试硬件目标

主要验证环境：

1.  Windows：NVIDIA RTX 4090 24GB + RTX 3060 12GB，64GB RAM；
2.  Ubuntu：AMD Radeon AI PRO R9700 32GB，96GB RAM；
3.  macOS：Apple Silicon 设备，验证 Metal 和统一内存；
4.  CPU-only：确保基础兼容路径。

硬件不可用的 CI 环境必须运行 Mock/Simulator
测试，并明确标记为非硬件验证。不能把模拟测试报告成真实 GPU 测试。

## 14.3 性能报告

每次基准测试固定：

-   模型仓库和 revision；
-   GGUF 文件和量化；
-   Runtime 版本；
-   Driver 版本；
-   GPU 设备；
-   Context；
-   Batch；
-   Prompt；
-   输出 Token 数；
-   并发任务数。

报告：

-   首 Token 延迟；
-   Prefill tok/s；
-   Decode tok/s；
-   总耗时；
-   每卡显存峰值；
-   RAM 峰值；
-   NVMe 读取量；
-   P2P 带宽；
-   错误和重试次数。

------------------------------------------------------------------------

# 15. 发布与运行目录

建议应用数据目录遵循平台习惯：

-   Windows：`%LOCALAPPDATA%/GGUF-AI-Engine/`；
-   Linux：`~/.local/share/gguf-ai-engine/`，配置放
    `~/.config/gguf-ai-engine/`；
-   macOS：`~/Library/Application Support/GGUF AI Engine/`。

用户可以在 UI 中将模型目录迁移到其他磁盘。

运行目录示意：

``` text
GGUF-AI-Engine/
├── app
├── runtimes/
├── models/
├── cache/
├── database/
├── logs/
└── config/
```

主程序可作为单文件入口，但 CUDA/ROCm/Metal 依赖、Runtime Worker
和模型文件应独立管理。不要承诺将所有驱动和模型塞入一个可执行文件。

------------------------------------------------------------------------

# 16. 关键架构决策记录（ADR）

## ADR-001：Rust Core + 三套原生 UI

决定：采用 Rust Core，WinUI 3、GTK4/Libadwaita、SwiftUI/AppKit 分别实现
UI。

理由：保留平台原生体验，同时共享核心业务逻辑。

## ADR-002：GGUF-first

决定：优先管理 GGUF，但允许 Manifest 声明必要的其他格式组件。

理由：多模态生态并非所有组件都能统一为 GGUF。

## ADR-003：Runtime Adapter

决定：不自研推理 Kernel，先集成成熟 Runtime。

理由：减少重复实现和硬件兼容风险。

## ADR-004：多 GPU capability-driven

决定：每种 Runtime 明确声明 Layer Split、Tensor Parallel、P2P 等能力。

理由：不同 GPU 和 Runtime 的并行能力并不相同。

## ADR-005：Runtime-managed 优先于 Engine-managed Paging

决定：优先使用 Runtime 原生
mmap/Offload/Paging；只有暴露安全接口时才自研 Tensor Pager。

理由：避免两个内存管理器争抢 Tensor 生命周期。

## ADR-006：NVMe 是冷层

决定：NVMe 用于模型持久化、冷权重和受控预读，不作为等价显存。

理由：IO 延迟和带宽与 VRAM 有数量级差距。

## ADR-007：Core 与 Worker 隔离

决定：推理 Runtime 作为独立进程。

理由：模型或驱动崩溃不能带崩桌面应用和管理服务。

------------------------------------------------------------------------

# 17. 交付给代码代理的最终任务

请按以下顺序实际实施，不要一次性生成所有功能：

### Task 1 --- Repository Bootstrap

-   创建 Cargo Workspace；
-   建立 crates 和 apps；
-   配置 lint、format、test；
-   添加 CI；
-   创建架构文档和 ADR。

### Task 2 --- Protocol + Core

-   定义 Protobuf；
-   建立 Core Daemon；
-   实现健康检查；
-   实现平台 IPC；
-   建立事件系统。

### Task 3 --- Native UI Shells

-   Windows WinUI 3 壳；
-   Linux GTK4 壳；
-   macOS SwiftUI 壳；
-   三者连接 Core；
-   实现 Overview 和 Settings 初始页面。

### Task 4 --- Hardware Inventory

-   CPU/RAM/Storage；
-   NVIDIA Probe；
-   AMD Probe；
-   Apple Metal Probe；
-   GPU topology；
-   capability status。

### Task 5 --- GGUF + Model Hub

-   GGUF Parser；
-   Hugging Face search；
-   下载；
-   校验；
-   Manifest；
-   SQLite Registry。

### Task 6 --- llama.cpp Adapter

-   Runtime 安装与版本管理；
-   模型加载；
-   Chat；
-   流式输出；
-   CPU/GPU Offload；
-   错误恢复。

### Task 7 --- Multi-GPU + Memory

-   GPU Scheduler；
-   Layer Split；
-   Task Parallel；
-   Resource Lease；
-   RAM/NVMe 管理；
-   监控和基准。

### Task 8 --- Multimodal Runtime Plugins

逐个增加 Image、Video、Audio、Music Adapter，并为每个 Adapter
建立真实兼容性测试。

### Task 9 --- Packaging

构建 Windows、Linux、macOS 安装包和升级流程。

**每完成一个
Task，先运行格式化、静态检查、单元测试和可执行的集成测试，再进入下一
Task。** 在硬件相关能力上，必须区分 Mock 测试和真实硬件测试。

------------------------------------------------------------------------

# 18. 完成定义（Definition of Done）

只有满足以下条件，才能宣称某项能力完成：

-   有真实实现，不是 UI 假数据；
-   有自动化测试；
-   有错误处理和日志；
-   有资源预算和释放；
-   有平台兼容性说明；
-   有 Runtime capability 声明；
-   有用户可理解的失败原因；
-   不会静默执行不可信代码；
-   文档与实际行为一致。

**V0.1 的首要成功标准：** 用户启动原生桌面应用，Core
自动检测本机硬件，用户可以从 Hugging Face 下载并校验一个 GGUF 模型，加载
llama.cpp Runtime，完成本地流式文本推理，并在 UI 中看到真实 GPU/RAM
使用情况。

这应当是整个项目的第一个端到端里程碑。
