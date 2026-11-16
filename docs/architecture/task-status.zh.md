> [English](task-status.md) | **中文**

# 实现状态

跟踪 spec §17 中的路线图。状态变更与本次更新在同一个提交里完成。

图例：**完成** —— 已实现、已测试、可用。**部分完成** —— 脚手架存在，并如实说明缺什么。
**计划中** —— 除了一份有文档的 crate 外壳之外，什么都没有。

| Task | 状态 |
| --- | --- |
| 1 仓库引导 | **完成** |
| 2 协议 + 核心 | **完成** |
| 3 原生 UI 外壳 | **部分完成**（Windows 完成，Linux 已编写，macOS 计划中） |
| 4 硬件清单 | **完成**（NVIDIA 已在真实的双 GPU 机器上验证） |
| 5 GGUF + 模型中心 | **完成** |
| 6 llama.cpp 适配器 | **完成**（真实 GPU 推理已验证） |
| 7 多 GPU + 内存 | **完成**（基准测试工具待做） |
| 8 多模态运行时插件 | **图像/视频/ASR/TTS 已完成**（其中三项端到端验证）；音乐没有运行时 |
| 9 打包 | **部分完成**（Windows MSIX 已构建；Linux/macOS 有配方；迁移 + 崩溃日志已完成） |

## Task 1 —— 仓库引导：**完成**

| 交付物 | 状态 | 证据 |
| --- | --- | --- |
| Cargo 工作区（apps + crates） | 完成 | `Cargo.toml`、`cargo build --workspace` |
| Lint / 格式化 / 测试配置 | 完成 | `[workspace.lints]`、`rustfmt.toml`、CI 门禁 |
| CI 流水线 | 完成 | `.github/workflows/ci.yml`（fmt、clippy、3 个操作系统上的测试、MSRV、unsafe 守卫） |
| 架构文档 + ADR | 完成 | `docs/architecture/`、`docs/adr/` |
| 提交 `Cargo.lock` | 完成 | 工作区根目录 |

## Task 2 —— 协议 + 核心：**完成**

| 交付物 | 状态 | 证据 |
| --- | --- | --- |
| Protobuf schema | 完成 | `proto/*.proto` |
| 生成的 gRPC 桩 | 完成 | `crates/protocol/build.rs`、内建的 `protoc` |
| 能力协商 | 完成 | `gguf_engine_protocol::features()`，由测试断言 |
| 核心守护进程 | 完成 | `apps/core-daemon`，带 IPC + 数据库达到 `ready` |
| 健康检查 | 完成 | 通过 gRPC 的 `HealthService`；`gguf doctor` 使用它 |
| 平台 IPC | 完成 | 命名管道（DACL：用户 + SYSTEM）/ Unix socket（0700 目录） |
| 事件系统 | 完成 | `EventService` 流式，带有界重放和按序号续传 |
| SQLite + 迁移 | 完成 | `crates/database`、`migrations/0001_init.sql`（19 张表） |
| 安全默认值 | 完成 | `crates/security`、ADR-010 |

## Task 3 —— 原生 UI 外壳：**部分完成**

| 外壳 | 状态 | 证据 |
| --- | --- | --- |
| Windows（WinUI 3，C#/XAML） | **已实现并测试** | `dotnet build` 无告警；20 个 xunit 测试，其中包括通过真实命名管道对着真实 `gguf-core` 的端到端测试 |
| Linux（GTK4，gtk4-rs） | **已构建、已验证，聊天已实现** | `platforms/linux/verify.sh`：真实核心 + Xvfb，窗口存活 8 秒，无严重错误。6 个聊天逻辑测试 |
| macOS（SwiftUI，AppKit） | **计划中** | `platforms/macos/README.md` 说明了缺什么，以及为什么没有交付代码 |

核心只产出一份展示快照（`proto/ui.proto`、`UiService/GetSnapshot`），外壳只负责渲染它
（ADR-012）。`gguf overview` 在终端里渲染同一份快照，这也是在没有显示器的情况下验证该契约的
方式。

## Task 4 —— 硬件清单：**完成**

| 交付物 | 状态 | 证据 |
| --- | --- | --- |
| CPU / 内存 / 存储 | 完成 | `crates/hardware/src/host.rs`（sysinfo） |
| NVIDIA 探测 | 完成 | `nvidia-smi` CSV + 拓扑；**已在 RTX 4090 + RTX 3060 上验证** |
| AMD 探测 | **实验性** | `rocm-smi --json`；按文档中的 schema 解析，从未在 AMD 硬件上运行过，并且会自报 `Experimental` |
| Apple Metal 探测 | 已编写，**未验证** | `system_profiler`；统一内存已处理（不伪造 VRAM） |
| GPU 拓扑 | 完成 | `nvidia-smi topo -m`；P2P 分为已检测 / 驱动支持 / 已测试 |
| 能力状态 | 完成 | 每个子系统都有 `ProbeOutcome`；`Unknown` 永不强制转换 |
| `HardwareService` + CLI | 完成 | `gguf hardware`、Overview 磁贴、e2e 测试 |

在本机上驱动拒绝打印拓扑矩阵，因此产品报告的是**未知，并附上驱动自己的信息**，而不是“没有 P2P”
—— 见 [hardware-inventory.md](hardware-inventory.zh.md)。

## Task 5 —— GGUF + 模型中心：**完成**

| 交付物 | 状态 | 证据 |
| --- | --- | --- |
| GGUF 解析器 | 完成 | `crates/gguf`；有界前缀读取，无 mmap，无 unsafe；35 个测试（20 个畸形输入 + 真实文件） |
| Hugging Face 搜索 | 完成 | `HubClient::search`；已实机验证 |
| 下载 + 续传 | 完成 | `Range` 续传，原子重命名；已实机验证 |
| SHA-256 校验 | 完成 | 就绪前校验；损坏的哈希被拒绝，不留下任何残留 |
| Manifest | 完成 | 由字节 + 仓库事实推导，绝不依据文件名 |
| 兼容性报告 | 完成 | 对未加载的模型如实给出 `compatible_unverified` |
| SQLite 注册表 | 完成 | `crates/database/registry.rs`；事务化 |
| CLI | 完成 | `gguf inspect`、`gguf models search/list/pull` |

真实运行：拉取 `tinyllamas/stories260K.gguf`，校验 SHA-256，原子提交，注册它，然后 `inspect`
报告从字节中解析出的真实架构。见 [gguf-and-model-hub.md](gguf-and-model-hub.zh.md)。

## Task 6 —— llama.cpp 适配器：**完成**

| 交付物 | 状态 | 证据 |
| --- | --- | --- |
| 运行时定位/探测 | 完成 | PATH、`GGUF_LLAMA_CPP_DIR`、已知目录；后端来自随附的 DLL，CUDA 设备来自 `--list-devices` |
| 模型加载 | 完成 | 启动 llama-server，等待 `/health`，捕获 stderr |
| 聊天 | 完成 | prompt 由历史折叠而来，走 `/completion` |
| 流式 | 完成 | SSE 解析为 token 流 |
| CPU/GPU 卸载 | 完成 | `--n-gpu-layers`；CUDA 已在本机验证 |
| 错误恢复 | 完成 | 工作进程崩溃表现为带 stderr 的 `WorkerExited`；核心存活（ADR-007） |
| CLI | 完成 | `gguf runtimes`、`gguf chat` |

```text
$ gguf runtimes
llama.cpp
  backends    : cpu, cuda
  multi-gpu   : true       gpu offload : true
  [unsupported] engine_managed_paging - llama.cpp manages its own memory

$ gguf chat --model tinyllamas/stories260K.gguf --prompt 'Once upon a time'
 Anna, Anna, I'm ready to
```

这就是 spec §18 里的 V0.1 里程碑：一个已验证的模型被真实运行时加载，从真实的 GPU 硬件上流式
吐出真实 token。见 [ADR-016](../adr/ADR-016-llama-cpp-http-server.zh.md)。

## Task 7 —— 多 GPU + 内存：**完成**

| 交付物 | 状态 | 证据 |
| --- | --- | --- |
| 按设备的 GPU 预算 | 完成 | `GpuBudget::for_device`；VRAM 未知则可用量为零 |
| 调度器 / 规划器 | 完成 | `plan()` —— 纯函数，在真实双 GPU 配置上有 17 个测试 |
| 层切分 | 完成 | 当一张卡放不下但总量放得下时提出 |
| 任务并行 | 完成 | 按设备的预算相互独立；租约从不跨设备 |
| 张量并行 | 受门禁 | 只有当运行时声明它时才启用（目前从不） |
| 资源租约 | 完成 | 基于 TTL 且可回收，因此崩溃无法占住 GPU |
| 内存 / NVMe 层级 | 完成 | `TierBudget`、`Pressure`、`grant_prefetch`；NVMe 是冷层级 |
| 监控 | 完成 | `gguf memory` 显示每张卡的 VRAM、内存、NVMe 和压力 |
| 基准测试 | **未实现** | 见下文 |

```text
$ gguf plan --weights-gib 20
GPU BUDGETS
  NVIDIA GeForce RTX 4090  usable 19.5 GiB of 22.0 GiB free (reserve 2.0 + margin 0.5)
  NVIDIA GeForce RTX 3060  usable  9.3 GiB of 11.8 GiB free (reserve 2.0 + margin 0.5)
                           total usable 28.8 GiB (never treated as one pool)
strategy : layer_split
  00000000:01:00.0 <- 19.5 GiB
  00000000:07:00.0 <-  2.2 GiB

$ gguf memory
  gpu_vram (RTX 4090)  19.5 GiB usable of 24.0 GiB (memory, reserve 2.5 GiB)
  host_ram             38.7 GiB usable of 63.8 GiB (memory, reserve 8.0 GiB)
  nvme (C:\)          542.4 GiB usable of 930.3 GiB (cold, reserve 10.0 GiB)
pressure : nominal
```

### 本任务中刻意不做的部分

| 能力 | 状态 |
| --- | --- |
| 性能基准测试（tok/s、各策略的 VRAM 峰值） | 未实现；需要固定的模型 + prompt 集合，应归入 Task 9 的验证 |
| P2P 带宽测量 | 仍是 `Unsupported`；在测量之前 `p2p_tested` 保持 false |
| 引擎托管的张量分页 | 被门禁关掉（ADR-005）；llama.cpp 管理自己的内存 |
| 把方案接进 `gguf chat` | 适配器仍然接收显式的 `--n-gpu-layers`；在那里使用方案属于后续工作 |

## Task 8 —— 多模态运行时插件：**部分完成**（vision 已完成，其余仅声明）

| 模态 | 状态 | 证据 |
| --- | --- | --- |
| 文本 | **完成** | llama.cpp，Task 6 |
| 视觉（图像输入） | **完成，已在真实硬件上验证** | `llama-mtmd-cli` + `mmproj`；一个 27B 模型对一张真实图像回答了一个真实问题 |
| 图像生成 | 不支持 | 没有扩散适配器；`gguf modalities` 也是这么说的 |
| 视频 | 不支持 | 没有视频适配器 |
| ASR | 不支持 | 没有语音识别适配器 |
| TTS | 不支持 | 没有语音合成适配器 |
| 音乐 | 不支持 | 没有音乐生成适配器 |

```text
$ gguf modalities
modality           status         runtime                detail
text               supported      llama.cpp              streaming chat through llama-server
vision             supported      llama.cpp mtmd         image in, text out; needs an mmproj companion file
image_generation   unsupported    -                      no diffusion runtime adapter is implemented
video              unsupported    -                      no video runtime adapter is implemented
asr                unsupported    -                      no speech-recognition adapter is implemented
tts                unsupported    -                      no speech-synthesis adapter is implemented
music              unsupported    -                      no music-generation adapter is implemented

2 of 7 modalities are implemented in this build

$ gguf vision --model Qwen3.8-27B.gguf --mmproj mmproj-Qwen3.8-27B-Q8_0.gguf --image test.png
The shape in the image is **yellow**. It's a solid yellow circle set against a blue...
```

多模态 Manifest 把 `mmproj-*.gguf` 配套文件与它们的权重配对，并把 projector 标记为 `required`；
不相关的 projector 会被报告为孤儿，而不是被悄悄挂上去（ADR-018）。

### 本任务中刻意不做的部分

| 能力 | 原因 |
| --- | --- |
| 扩散 / 视频 / 音频 / 音乐适配器 | 没有安装或指定任何运行时；把它们标成 supported 会是一个假功能 |
| 常驻的视觉会话 | `llama-mtmd-cli` 是一次性的，所以每个请求都要重新加载模型；CLI 会说明这个代价 |
| 通过 gRPC API 使用视觉 | `InferenceService` 仍未实现（Task 9 后续） |

## Task 9 —— 打包：**部分完成**（Windows 已验证，Linux/macOS 有配方）

| 交付物 | 状态 | 证据 |
| --- | --- | --- |
| Windows MSIX | **已构建并验证** | `scripts/package-windows.ps1` → 69.8 MB 的包；`makeappx unpack` 成功并列出所有二进制 |
| Linux deb / rpm | **已构建、已安装、已启动** | 包内包含 GTK4 外壳；安装它并在 Xvfb 下运行桌面入口的 `Exec=` 路径可行 |
| macOS DMG | 只有配方 | `packaging/macos/README.md`；它要包装的那个外壳仍在计划中 |
| 签名 / 校验 | 有文档，未自动化 | 签名是发布决定（ADR-019）；脚本产出未签名的产物 |
| 自动更新 | **未实现** | 需要签名通道 + 回滚策略；做一半比不做更糟 |
| 运行时独立更新 | 仅设计 | 运行时按用户安装，位于包之外（spec §15） |
| 用户数据迁移 | **完成** | `Database::open_with_backup` 在迁移前备份，若复制失败则中止；6 个测试 |
| 崩溃日志 | **完成** | panic hook 写出 `crash-<ms>.log`；`gguf doctor` 按路径列出它们 |
| 文档 | 完成 | `packaging/README.md` 加上每个平台一份 README |

```text
$ ./scripts/package-windows.ps1
makeappx  : ...\10.0.26100.0\x64\makeappx.exe
version   : 0.1.0.0
shell     : published to target/package/stage/shell
package   : target/package/windows/GGUFAIEngine-0.1.0-x64.msix
size      : 69.8 MB
```

```text
$ gguf doctor
[ok  ] database           schema v2 (expected v2), sqlite 3.50.2
[ok  ] crash reports      none in ...\logs
```

### 本任务中刻意不做的部分

| 能力 | 原因 |
| --- | --- |
| Linux / macOS 安装包 | 没有机器可以构建它们；一个从未构建过的配方不算交付物 |
| 代码签名 | 需要项目没有的证书；把它自动化只会产出不可信的包 |
| 自动更新 | 需要签名通道、降级策略和回滚；数据库已经拒绝降级 |
| 运行时自动安装 | 运行时按设计就是按用户、彼此独立的（spec §15） |

## Task 9 之后 —— 通过 gRPC 聊天：**完成**

Task 6 给 CLI 带来了进程内聊天。UI 外壳需要的是通过核心获得同样的能力，这正是
`InferenceService` 现在提供的。

| 交付物 | 状态 | 证据 |
| --- | --- | --- |
| `InferenceService.Chat` | 完成 | 流式返回 `ChatChunk`；与外壳使用的是同一条路径 |
| 模型解析 | 完成 | 一个文件路径，或者一个已注册的仓库 id，解析到其最新修订的权重 |
| 断连即卸载 | 完成 | `Drop` 守卫即使在客户端回答途中消失时也会卸载 |
| 并发 | 一个模型 | 第二个请求会得到 `RESOURCE_EXHAUSTED`，而不是一个没有解释的等待 |
| `gguf ask` | 完成 | 通过 gRPC 经运行中的核心聊天 |
| 端到端测试 | 完成，需显式启用 | `chat_e2e` 流式返回真实 token，并拒绝未知模型 |

```text
$ gguf ask --model ggml-org/models --prompt 'Once upon a time there was' --max-tokens 20
 Anna was very excited and collecting
[20 chunks, 20 tokens, 32 ms, 625.0 tok/s]

$ gguf ask --model ggml-org/models --prompt 'The sky is' --max-tokens 12 --json
{"answer":" Anna, I'm scar","chunks":12,"elapsed_ms":16,"ok":true,
 "tokens_generated":12,"tokens_per_second":750.0}
```

有两种失败模式得到了处理，因为它们是测试发现的，而不是读代码读出来的：

- 客户端在流中途断开，否则会让一个 `llama-server` 占着 VRAM 直到核心重启，因此卸载逻辑放在
  一个 `Drop` 守卫里；
- 一个 server-streaming RPC 可能把失败作为流的第一个元素报告出来，而不是在调用时报错，所以测试
  两种都接受，但仍然拒绝空回答。

## Task 9 之后 —— 兼容 OpenAI 的网关：**完成**

| 交付物 | 状态 | 证据 |
| --- | --- | --- |
| `GET /health` | 完成 | `{"status":"ok"}` |
| `GET /v1/models` | 完成 | 读取真实注册表：`ggml-org/models` |
| `POST /v1/chat/completions` | 完成 | 非流式与 SSE 流式；真实 token |
| 安全默认值 | 完成 | 默认只监听回环；不带 token 的非回环绑定在 socket 打开之前就被拒绝 |
| CORS | 仅显式选择 | 除非运维要求，否则不发送任何头 |
| 测试 | 完成 | 15 个测试，其中几个跑在真实的回环 socket 上 |

```text
$ curl 127.0.0.1:18888/v1/models
{"object":"list","data":[{"id":"ggml-org/models","object":"model","owned_by":"llama"}]}

$ curl 127.0.0.1:18888/v1/chat/completions -d '{"model":"ggml-org/models",...}'
{"object":"chat.completion","choices":[{"message":{"role":"assistant",
  "content":" Anna, Anna, I'm "},"finish_reason":"stop"}],
 "usage":{"prompt_tokens":0,"completion_tokens":16,"total_tokens":16}}

unknown model -> 404 {"error":{"code":"model_not_found"}}
```

网关是建立在 `InferenceService` 之上的协议翻译器，而不是第二条推理路径（ADR-020）。
`prompt_tokens` 是 0，因为运行时不统计它们；一个猜测会显得很有权威，但其实是错的。

## 后续 —— Windows 外壳聊天：**完成**

| 交付物 | 状态 | 证据 |
| --- | --- | --- |
| 聊天面板 | 完成 | 模型 id、prompt、对话记录、发送/停止 |
| 流式渲染 | 完成 | token 一到就出现；停止会保留部分回答 |
| 不依赖工具包的逻辑 | 完成 | `ChatSession` 和 `ChatTranscript` 在没有窗口的情况下做了单元测试 |
| C# 测试 | 完成 | 33 个通过（原来是 20 个） |

已用 UI Automation 对着一个运行中的核心验证：窗口暴露了 Chat 导航项，并渲染出真实的值
（`2 device(s), 36.0 GiB VRAM`、`Intel(R) Core(TM) i9-14900KF`、
`24 physical / 32 logical cores`）。

`crates/inference` 已被删除：它只是个占位符，真实实现位于 `crates/ipc/src/inference.rs`。

## 后续 —— Linux 外壳已验证：**完成**

首次在 Linux 上构建 GTK4 外壳时，发现了两个只在 Windows 上跑的门禁永远看不到的缺陷：

| 缺陷 | 后果 |
| --- | --- |
| `crates/ipc/src/client.rs` 中 `#[cfg(unix)]` 的连接器在需要 `'static` 的地方捕获了一个借用的 `&str` | Unix socket 客户端**根本编译不过** |
| 用到了 `glib`，但从未在 `platforms/linux/Cargo.toml` 中声明 | `MainContext::spawn_local` 无法解析 |

验证现在是可执行的，而不是口头断言：

```text
$ bash platforms/linux/verify.sh
== starting the core
   core ready
== launching the shell under Xvfb for 8 seconds
   shell stayed alive and was stopped by the timeout
== errors logged by the shell
   none
== PASS
```

`verify.sh` 把退出码 124（超时杀掉了仍在运行的应用）视为成功，其他任何情况都算失败，
所以一个启动后就死掉的外壳不可能通过。

## 后续 —— embeddings：**完成**

| 交付物 | 状态 | 证据 |
| --- | --- | --- |
| `InferenceService.Embeddings` | 完成 | 新 RPC；以 embedding 模式加载工作进程 |
| `POST /v1/embeddings` | 完成 | OpenAI 形状；字符串或数组输入 |
| 如实拒绝 | 完成 | 无法以 embedding 模式运行的模型返回 `FAILED_PRECONDITION`，而不是一个无意义的向量 |

```text
$ curl 127.0.0.1:18888/v1/embeddings -d '{"model":"ggml-org/models","input":["hello","world"]}'
object: list  count: 2  vector[0] length: 64
first 3 values: 1.0292757, -1.3270983, 0.1801005
```

llama.cpp 在其原生 `/embedding` 端点上暴露 embeddings（每个请求一个输入），而它的
`/v1/embeddings` 在这个构建上返回 400，因此网关做翻译，并且每个输入发一个请求，而不是假装
批处理。

## 后续 —— Linux 包已构建：**完成**

配方不再只是一句声称。`packaging/linux/build-packages.sh` 产出两种格式，然后对照 README
的承诺校验 .deb 的文件列表。

```text
created package: dist/gguf-ai-engine_0.1.0_amd64.deb      (12.1 MB)
created package: dist/gguf-ai-engine-0.1.0-1.x86_64.rpm  (12.4 MB)
== verifying the .deb file list
   all expected files present
```

然后安装并运行，这是唯一能证明一个包可用的方式：

```text
$ gguf version --json      -> { "os": "linux", "protocol": 2 }
$ gguf doctor              -> ipc: /tmp/.../run/core.sock, database schema v2, core ready
$ dpkg -r gguf-ai-engine
   user data survived the uninstall
```

doctor 的输出还证明了 **Unix socket IPC 传输**可用：正是这一轮之前编译不过的那条代码路径。

做这件事时发现的一个缺陷：`nfpm` 会展开包元数据里的 `{{ .Env.VERSION }}`，却把原始模板写进
文件名，产出 `gguf-ai-engine_{{ .Env.VERSION }}_amd64.deb`。脚本现在自己解析版本，并且如果生成的
配置里还残留任何模板就拒绝运行。

## 后续 —— Linux 聊天页面：**完成**

Linux 外壳现在拥有与 Windows 外壳相同的聊天能力：模型 id、prompt、对话记录、发送 / 停止 / 清空。

这些规则位于 `platforms/linux/src/chat.rs`，不依赖 GTK，因此无需显示器即可测试：

| 规则 | 原因 |
| --- | --- |
| 空 prompt 在到达核心之前就被拒绝 | 否则会为了回答空内容而加载一个模型 |
| 待发送的 prompt 加入请求，但不加入对话记录 | 失败的调用绝不能让历史里出现一个从未发生过的回合 |
| 空的增量不是 token | 运行时把它们当心跳发送 |
| 停止会保留部分回答 | 用户是看着那些 token 到达的 |

中止流式任务会丢弃该流，这正是通知核心卸载模型的方式 —— 与 gRPC 服务执行的“断连即卸载”是同
一条规则。

```text
$ cd platforms/linux && cargo test
test result: ok. 6 passed; 0 failed
```

## 后续 —— Linux 包带上 GUI：**完成**

.deb 打包了 CLI 和核心，却没有打包外壳，于是桌面入口指向一个不存在的路径。它被记录为“在 GTK4
外壳被打包之前处于惰性状态” —— 这句话在外壳构建成功之后就不再成立，并在有人安装它的那一刻变成
了谎言。

`build-packages.sh` 现在也会构建 `platforms/linux`，并把结果安装到桌面入口所指的确切路径。
包声明了 `Depends: libgtk-4-1`，链接系统 GTK，而不是捆绑第二份拷贝。

通过安装 .deb 并启动该路径验证：

```text
-rwxr-xr-x 4194024 /usr/lib/gguf-ai-engine/gguf-ai-engine-ui
Exec=/usr/lib/gguf-ai-engine/gguf-ai-engine-ui
   core ready
   packaged UI stayed alive for 8s
   no critical errors
== PASS
```

## 后续 —— 四个模态：**已实现**

Task 8 要求图像、视频、音频和音乐适配器。三个运行时从源码构建并被真实驱动；音乐没有任何运行时，
报告也如实这么说。

| 运行时 | 模态 | 已验证 |
| --- | --- | --- |
| stable-diffusion.cpp（`sd-server`，CUDA 13.4） | 图像生成 | **是** —— 512x512 PNG，14.2 秒 |
| stable-diffusion.cpp | 视频 | **是** —— Wan 2.1 T2V 1.3B：经适配器 17 帧用时 95.7 秒；拒绝路径也已验证 |
| whisper.cpp（`whisper-cli`，CUDA） | ASR | **是** —— 通过 SAPI 语音做了一次往返 |
| Piper | TTS | **是** —— 而且它的输出能转写回同一句话 |
| audio.cpp（ggml，GGUF） | 音乐、音效 | **是** —— 在 CUDA 上 1.6 秒生成 8 秒音乐 |

```text
$ gguf modalities
asr                supported    whisper.cpp            provided by whisper.cpp (...whisper-cli.exe)
image_generation   supported    stable-diffusion.cpp   provided by stable-diffusion.cpp (...sd-server.exe)
music              unsupported  -                      no runtime in this build provides music
text               supported    llama.cpp              streaming chat through llama-server
tts                supported    Piper                  provided by Piper (...piper.exe)
video              supported    stable-diffusion.cpp   provided by stable-diffusion.cpp
vision             supported    llama.cpp mtmd         image in, text out

6 of 7 modalities are implemented in this build
```

### 让这一切成为可能的架构

`crates/multimodal` 给每个运行时一份 **Manifest**（spec §2.3：OS、架构、后端、模型架构、量化、
并行模式、卸载、分页、模态）、一个找到其二进制并说明在哪里找过的**探测**，以及一个会说其协议的
**适配器**。模态注册表随后由探测推导而来，而不是一份手写的清单：一项能力之所以提供，是因为找到了
对应的运行时；缺失的那项则带有安装提示。

### 一次改变了结论的测量

第一张图花了 412.9 秒。这对一块 4090 来说不合常理，所以它被调查了，而不是被直接上报：先前测试
遗留的一个 `llama-server` 占着这张 24 GB 卡中的 21 GB，把它杀掉后同一次渲染变成了 **14.2 秒**
—— 相差 29 倍。这正是 GPU 管理器的按设备预算和会过期的租约所要防止的失败，只不过这次是从外部
看到的。

### 刻意不做的部分

| 能力 | 原因 |
| --- | --- |
| ~~音乐生成~~ | **完成** —— audio.cpp、GGUF，不需要 Python，也不需要 ComfyUI |
| 用 Wan 2.2 A14B 渲染 | 它需要两个 13.3 GB 的 transformer 再加一个 6.3 GB 的编码器；能放进 24 GB 卡的是 1.3B 变体 |
| 自动启动扩散服务器 | 模型必须出现在它的命令行上；引擎指向用户自己运行的服务器 |
| UI 外壳里的 `/v1/audio` | 端点已经存在；外壳还没有音频页面 |

## 后续 —— 外壳拿到媒体服务：**完成（Windows）**

Create 页面通过外壳已经在用的同一种 gRPC 传输来驱动 `MediaService`，因此没有任何外壳需要长出
一个 HTTP 客户端。

| 组件 | 状态 |
| --- | --- |
| `proto/media.proto` + `MediaService` | 完成：图像、视频、语音、转写 |
| `crates/multimodal::MediaEngine` | 完成：gRPC 与 HTTP 网关共用一份实现 |
| `crates/ipc/src/media.rs` | 完成：运行时缺失 -> 带探测原因的 `FAILED_PRECONDITION` |
| Windows Create 页面 | 完成：图像、视频（带进度）、音乐、语音 |
| Linux Create 页面 | **未完成** |

在选中该页面后用 UI Automation 验证（折叠的面板不在自动化树里，所以选择页面本身就是检查的一部分）：

```text
elements after selecting Create: 32
  [ok] Create   [ok] Image    [ok] Speech   [ok] Generate   [ok] Synthesise
  [ok] Describe the image...   [ok] Text to speak...   [ok] Width   [ok] Height
```

## 后续 —— 视频进度流：**完成**

`GenerateVideo` 现在是 server-streaming（`VideoProgressEvent`），因为一个片段要花几分钟，
而阻塞调用无法区分“正在工作”和“卡住了”。

| 层 | 状态 |
| --- | --- |
| 适配器里的 `VideoProgress` | 完成 —— 阶段、详情、已完成/总数、单位、已用时间、队列位置 |
| `multimodal::video::generate_with_progress` | 完成 —— 只有一份实现，`generate` 委托给它 |
| `MediaEngine::video_with_progress` | 完成 |
| gRPC 流式 | 完成 —— 渲染跑在自己的任务里，并通过 channel 汇报 |
| CLI | 完成 —— 每个阶段变化一行，输出到 stderr |
| Windows 外壳 | 完成 —— ProgressBar + 状态行 |
| Linux 外壳 | **未完成** |

```text
$ gguf video --prompt 'a fox in a snowy forest' --steps 8
conditioning (2s)
sampling 0/8 steps 0% (8s)
sampling 3/8 steps 38% (64s)
vae_decode (154s)
encoding (165s)
```

通过 Windows UI 做了端到端验证：UI Automation 设置 prompt，点击 Generate clip，并观察到
`sampling 3/8 steps` -> `vae_decode` -> `Wrote 424 KiB (17 frames)`。

做这件事时发现了两个真实缺陷：适配器从未发送 `sample_steps`（于是运行时用了默认的 20 步，
渲染时间变成三倍），以及 CLI 每次轮询都重写它的进度行，因为已用时间总在变。

## 后续 —— GGUF 上的音乐与音效：**完成**

`audio.cpp` 是一个 ggml 运行时（与 llama.cpp 同族）：GGUF 权重，纯 C++，不需要 Python，
不需要 ComfyUI。这里用 CUDA 从源码构建，并使用一个受限的模型集合。

```text
$ gguf music --prompt 'warm ambient piano with soft rain' --duration 8
wrote song.wav (1.3 MiB, 44100 Hz, 2 channel(s))     1.6 s

$ gguf modalities | Select-String music
music              supported      AudioCPP      provided by AudioCPP (...audiocpp_cli.exe)
sfx                supported      AudioCPP      provided by AudioCPP (...audiocpp_cli.exe)
```

| 模态 | 模型 | 用时 |
| --- | --- | --- |
| 音乐 | Stable Audio 3 Small Music，f16，CUDA | 1.6 秒生成 8 秒音频 |
| 音效 | Stable Audio 3 Small SFX，f16，CUDA | 2.8 秒生成 5 秒音频 |

### 一个靠崩溃发现的量化/后端组合

第一次尝试音乐时，进程在 2.8 秒内死在 ggml 的 MMQ 内核里（`mmq_x_best=0`）。Q8_0 权重在 CUDA
上中止；Q8_0 在 CPU 上和 f16 在 CUDA 上都正常。因此后端成了配置项（`multimodal.audio_backend`），
文档也写明这一组合，而不是只说“用 CUDA”。

### 关于“一切都必须是 GGUF”

spec §1.2 和 ADR-002 说的是 **GGUF 优先，而非只用 GGUF**：GGUF 是首选，多模态模型可以声明必要的
非 GGUF 组件，只要 Manifest 列出了它们。这也是为什么 Manifest 携带 `model_archs` 和
`quantizations`：Piper 的声音是 ONNX，并且会说明这一点。

在这个产品真正弱于该规则的地方，问题不是格式 —— 而是来源：图像和视频的测试命令指向了本机一个
ComfyUI 安装里的权重。从来没有运行时调用过 ComfyUI，但一个写着 `E:\ComfyUI\models\...` 的路径
看起来就像依赖。音乐、语音和 Wan 视频模型现在都放在项目自己的运行时目录下；图像那条路径仍然从
那里借用一个 checkpoint，应当以同样的方式搬走。

## 后续 —— 外壳里的音乐，以及权重搬回家：**完成**

| 组件 | 状态 |
| --- | --- |
| `proto/media.proto` 的 `GenerateMusic` | 完成 |
| `crates/ipc/src/media.rs` | 完成 —— CPU 密集的 CLI 跑在 reactor 之外 |
| C# 客户端 + Create 页面 | 完成 —— 带 prompt 和时长的 Music 区块 |
| 项目树里的权重 | 完成 —— 图像、视频、音乐、语音 |

```text
$ gguf music --prompt 'soft ambient piano with rain' --duration 10
wrote song.wav (1.3 MiB, 44100 Hz, 2 channel(s))

# and through the shell, verified with UI Automation:
Wrote 1722 KiB (44100 Hz, 2 ch) to C:\...\gguf-music-172101.wav
```

搬走权重后验证：图像从 GGUF 的 SD1.5 出发，10.5 秒产出 481 KB 的 PNG；视频加载我们的 umt5
Q4_K_M（3479.75 MB）和我们的 VAE（242.10 MB）—— 整个视频栈驻留 6.4 GB 而不是 9.4 GB，并且没有
任何路径指向一个 ComfyUI 安装。

### 陈旧二进制陷阱，又踩了一次

从外壳调用 `GenerateMusic` 在 0 秒就返回 FAILED_PRECONDITION，而 CLI 却是好的。添加该 RPC
之后核心二进制没有被重新构建，所以在运行中的进程里这个服务确实未实现。
`cargo build -p gguf-engine-ipc` 构建的是库，不是 `gguf-core.exe`；守护进程需要自己构建。
值得记住：零秒就失败的 UI 问题，通常不是功能的问题。
