# 编译与使用手册

本文档面向两类读者：想从源码构建本项目的人，和想用它跑模型的人。
文中的命令都实际执行过，输出摘录自真实运行。

- 工程规格：[GGUF-AI-Engine-Engineering-Spec.md](../GGUF-AI-Engine-Engineering-Spec.md)
- 架构说明：[docs/architecture/](architecture/)
- 决策记录：[docs/adr/](adr/)

---

## 1. 这是什么

一个本地 AI 推理平台。管理 GGUF 模型，用**独立的 C++ 运行时**做推理，
通过原生界面、命令行和本地 API 使用。没有 Python 运行时依赖，也不经过 ComfyUI。

### 支持的模态

| 模态 | ID | 运行时 | 状态 |
| --- | --- | --- | --- |
| 文本对话 | `text` | llama.cpp（`llama-server`） | 可用 |
| 看图说话 | `vision` | llama.cpp mtmd | 可用 |
| 图像生成 | `image_generation` | stable-diffusion.cpp（`sd-server`） | 可用 |
| 视频生成 | `video_generation` | stable-diffusion.cpp | 可用 |
| 语音识别 | `speech_recognition` | whisper.cpp | 可用 |
| 语音合成 | `speech_synthesis` | Piper 或 audio.cpp | 可用 |
| 音乐生成 | `music` | audio.cpp（GGUF） | 可用 |
| 音效生成 | `sfx` | audio.cpp（GGUF） | 可用 |

`gguf modalities` 会按本机实际情况回答，缺哪个、缺什么它会说清楚。

---

## 2. 环境要求

### 通用

| 项 | 要求 |
| --- | --- |
| Rust | **1.93 或更高**（`rust-version` 在 `Cargo.toml` 里；CI 用 stable 验证） |
| Edition | 2024 |
| 磁盘 | 编译约 8 GB；模型另计（单个 2–20 GB） |

源码里禁止 `unsafe`（`[workspace.lints.rust] unsafe_code = "forbid"`），
唯一例外是 `crates/security` 的 Windows 管道 ACL（ADR-010）。

### Windows

| 项 | 要求 |
| --- | --- |
| 工具链 | MSVC（Visual Studio Build Tools 2022+，C++ 桌面负载） |
| 界面 | .NET SDK 8（WinUI 3 外壳） |
| 运行 | Windows 10 1809+ |

### Linux

| 项 | 命令 |
| --- | --- |
| 编译工具 | `sudo apt install build-essential pkg-config` |
| GTK4 外壳 | `sudo apt install libgtk-4-dev` |
| 打包 | `go install github.com/goreleaser/nfpm/v2/cmd/nfpm@latest` |

### macOS

核心可以编译（CI 在 macOS 上跑全部测试）。SwiftUI 外壳尚未实现，
见 [platforms/macos/README.md](../platforms/macos/README.md)。

---

## 3. 快速开始

```bash
# 1. 编译
cargo build --release

# 2. 看看这台机器有什么
./target/release/gguf hardware

# 3. 看看哪些模态能用
./target/release/gguf modalities

# 4. 拉一个模型
./target/release/gguf models search "qwen2.5 3b gguf"
./target/release/gguf models pull <repo>/<file.gguf>

# 5. 直接对话（不需要守护进程）
./target/release/gguf chat --model <模型路径> --prompt "你好"
```

想要图形界面或给别的程序用 API，再启动守护进程：

```bash
./target/release/gguf serve        # 前台运行，Ctrl-C 停止
```

---
## 4. 编译

### 4.1 Rust 工作区（核心）

```bash
cargo build --release          # 产出 target/release/{gguf, gguf-core, gguf-worker}
cargo build                    # debug 版，测试用这个
```

三个可执行文件：

| 文件 | 作用 |
| --- | --- |
| `gguf` | 命令行客户端 |
| `gguf-core` | 核心守护进程：数据库、IPC、HTTP 网关 |
| `gguf-worker` | 隔离的运行时工作进程 |

**编译顺序很重要**：端到端测试会真的启动 `gguf` 和 `gguf-core`。
只跑 `cargo test` 而不先 `cargo build`，测试可能拿到旧的二进制，
表现为「明明修好了却还失败」。CI 因此把 `cargo build --workspace` 放在测试之前。

### 4.2 Windows 外壳（WinUI 3）

```powershell
cd platforms\windows\GgufAiEngine.Shell
dotnet restore
dotnet build -c Release -p:Platform=x64
```

产物：`bin\x64\Release\net8.0-windows10.0.19041.0\win-x64\GgufAiEngine.Shell.exe`

外壳的 gRPC 存根（`Generated/*.cs`）是**提交进仓库**的，因为 WinUI 的
XAML 预编译阶段早于任何能生成它的构建目标。改了 `proto/` 就要重新生成：

```powershell
./scripts/generate-csharp-protos.ps1           # 重新生成
./scripts/generate-csharp-protos.ps1 -Check    # 只校验，CI 用这个
```

脚本从 NuGet 的 Grpc.Tools 取 protoc，所以**必须先 `dotnet restore`**。

### 4.3 Linux 外壳（GTK4）

```bash
sudo apt install -y libgtk-4-dev pkg-config
bash platforms/linux/build.sh --release
```

无显示器环境下验证窗口能起来：

```bash
sudo apt install -y xvfb
bash platforms/linux/verify.sh
```

`build.sh` 默认把产物放到 `$HOME/target-linux`——源码放在 9p 挂载（例如
Windows 共享盘）上时，本地文件系统上的 target 目录快好几倍。

### 4.4 Linux 安装包（.deb / .rpm）

```bash
bash packaging/linux/build-packages.sh
```

这个脚本做完整条链：读版本号 → 编译 release 二进制 → 编译 GTK4 外壳 →
生成 `.deb` 和 `.rpm` → 校验包内文件清单。实测输出：

```text
== building the GTK4 shell
    Finished `release` profile [optimized] target(s) in 55.17s
== building .deb
created package: dist/gguf-ai-engine_0.1.0_amd64.deb
== building .rpm
created package: dist/gguf-ai-engine-0.1.0-1.x86_64.rpm
== verifying the .deb file list
   all expected files present
== PASS
```

包内布局：

| 路径 | 内容 |
| --- | --- |
| `/usr/bin/gguf` | 命令行入口（包装脚本） |
| `/usr/lib/gguf-ai-engine/gguf` | 客户端 |
| `/usr/lib/gguf-ai-engine/gguf-core` | 守护进程 |
| `/usr/lib/gguf-ai-engine/gguf-worker` | 工作进程 |
| `/usr/lib/gguf-ai-engine/gguf-ai-engine-ui` | GTK4 外壳 |
| `/usr/share/applications/gguf-ai-engine.desktop` | 桌面入口 |
| `/usr/share/doc/gguf-ai-engine/README.md` | 用户文档 |
| `/usr/lib/gguf-ai-engine/runtimes` | 空目录，运行时装在这里 |

包**不包含**模型和运行时——那些按用户、按需下载，卸载时用户数据保留。

---
## 5. 运行时

引擎本身不含推理内核：它集成成熟的 C++ 运行时（ADR-003）。
每个运行时是一个独立进程或命令行工具，需要你自己装。

### 5.1 查找规则

运行时会到这些地方找：

1. 环境变量 `GGUF_ENGINE_RUNTIME_DIR` 指定的目录（**分号分隔**，可多个）
2. 平台约定目录：`%LOCALAPPDATA%\gguf-ai-engine\runtimes`、`~/.local/share/gguf-ai-engine/runtimes`
3. `PATH`

llama.cpp 额外认 `GGUF_LLAMA_CPP_DIR`。

```bash
# 例：一次性把四个运行时目录都告诉引擎
export GGUF_ENGINE_RUNTIME_DIR="/opt/sd.cpp/bin:/opt/whisper.cpp/bin:/opt/piper:/opt/audio.cpp/bin"
```

### 5.2 五个运行时

| 运行时 | 提供 | 可执行文件 | 模型格式 |
| --- | --- | --- | --- |
| llama.cpp | 文本、视觉 | `llama-server` | GGUF |
| stable-diffusion.cpp | 图像、视频 | `sd-server` | GGUF / safetensors |
| whisper.cpp | 语音识别 | `whisper-cli` | ggml `.bin` |
| Piper | 语音合成 | `piper` | ONNX + 音色 |
| audio.cpp | 音乐、音效、GGUF 语音 | `audiocpp_cli` | GGUF |

查本机装了什么、每个运行时声称支持什么：

```bash
./target/release/gguf runtimes
```

### 5.3 为什么扩散服务要单独启动

图像和视频走 `sd-server` 的 HTTP 接口，**由你自己启动**，而不是引擎每请求起一个进程。
原因有两个：模型必须写在它的命令行上，而加载一次要几十秒。

```bash
# 图像（SD 1.5）
sd-server --model /models/sd1.5-q8_0.gguf --listen-ip 127.0.0.1 --listen-port 1234

# 视频（Wan 2.1 T2V 1.3B，需要文本编码器和 VAE）
sd-server --diffusion-model /models/wan2.1_t2v_1.3B.safetensors \
          --t5xxl /models/umt5-xxl-encoder-Q4_K_M.gguf \
          --vae /models/wan_2.1_vae.safetensors \
          --listen-port 1234
```

然后在配置里指向它（见下一节 `diffusion_url`）。

### 5.4 后端与量化必须配对

在 RTX 4090 + 驱动 610.88 上实测 audio.cpp：

| 权重 | 后端 | 结果 |
| --- | --- | --- |
| q8_0 | cuda | **进程崩溃**（ggml MMQ kernel 断言 `mmq_x_best=0`） |
| q8_0 | cpu | 正常 |
| f16 | cuda | 正常 |

所以 `audio_backend` 是配置项，而且文档必须写「配对」而不只是「用 CUDA」。

---

## 6. 配置

### 6.1 文件位置

| 平台 | 数据根目录 |
| --- | --- |
| Windows | `%LOCALAPPDATA%\GGUF-AI-Engine\` |
| Linux | `~/.local/share/gguf-ai-engine/` |
| macOS | `~/Library/Application Support/GGUF AI Engine/` |

配置文件名 `config.toml`，放在数据根目录下。

覆盖顺序：`--data-dir` > 环境变量 `GGUF_AI_ENGINE_DATA_DIR` > 平台约定。
配置文件路径用 `--config` 覆盖。

```bash
./target/release/gguf config          # 打印解析后的配置和校验结果
./target/release/gguf config --json   # 给程序读
```

### 6.2 默认值

```text
[core]
  host / api_port / grpc_port : 127.0.0.1:18888 / 18889
  log_level                    : info
[memory]
  gpu_reserve_bytes            : 2147483648 (2.0 GiB)
  ram_reserve_bytes            : 8589934592 (8.0 GiB)
  nvme_reserve_bytes           : 10737418240 (10.0 GiB)
  max_prefetch_bytes           : 2147483648 (2.0 GiB)
[api]
  bind / authentication        : 127.0.0.1 / LocalOnly
validation  : ok
```

保留量不是摆设：显存预算按**每张卡**算，任何一张都不许被计划到低于保留值。

### 6.3 多模态配置

```toml
[multimodal]
# 图像与视频共用的扩散模型
image_model = '/models/sd1.5-q8_0.gguf'
# 已启动的扩散服务地址；留空则用运行时默认端口
diffusion_url = 'http://127.0.0.1:1234'
# whisper.cpp 模型
asr_model = '/models/ggml-tiny.bin'
# Piper 音色（ONNX）。设置 tts_model 后改用 GGUF 语音
tts_voice = '/voices/en_US-amy-low.onnx'
# audio.cpp 音乐模型【目录】，不是文件
music_model = '/models/audio/stable-audio-3-small-music-f16'
# 设置这一项即从 Piper 切到 GGUF 语音（Qwen3 TTS）
# tts_model = '/models/audio/qwen3-tts-customvoice'
# audio.cpp 的后端：cpu 或 cuda（注意与量化的配对）
audio_backend = 'cuda'
```

注意 `music_model` 和 `tts_model` 是**目录**：audio.cpp 会在目录里解析组件，
传 `.gguf` 文件路径它会报找不到组件。

---
## 7. 命令行

全局选项对每个子命令都有效：

```text
--data-dir <DIR>   覆盖数据目录
--config <FILE>    覆盖配置文件
--json             输出 JSON（脚本友好）
-v / -vv / -vvv    日志详细度
```

### 7.1 诊断

```bash
gguf doctor                  # 环境体检：目录、磁盘、锁、数据库、运行时、崩溃报告
gguf hardware                # CPU / 内存 / 存储 / 每张 GPU
gguf overview                # 原生界面显示的那份快照
gguf version --json
```

`gguf doctor` 是出问题时的第一条命令：它逐项说明「检查了什么、结果如何、
为什么不可用」，而不是只给一个 pass/fail。

### 7.2 模型

```bash
gguf models search "qwen2.5 gguf"     # 搜 Hugging Face
gguf models pull <repo> <file.gguf>   # 下载、校验、登记
gguf models list                      # 本地登记表
gguf inspect <file.gguf>              # 解析 GGUF，报告架构/量化/张量/元数据
```

`inspect` 是纯解析，不加载模型，几毫秒就能告诉你这个文件是什么、能不能跑。

### 7.3 推理（直接调用运行时，不需要守护进程）

```bash
gguf chat --model <路径> --prompt "解释一下 GGUF" --max-tokens 256
gguf vision --model <路径> --image photo.jpg --prompt "描述这张图"
```

`chat` 是流式输出，token 边生成边打印。

### 7.4 图像

```bash
gguf image --prompt "a red fox in snow" --out fox.png --width 512 --height 512 --steps 20
```

实测（SD 1.5 GGUF，RTX 4090）：

```text
wrote fox.png (481393 bytes)      # 512x512，10.5 秒
```

### 7.5 视频

```bash
gguf video --prompt "a lighthouse on a cliff at sunset" \
           --out clip.mp4 --frames 17 --fps 8 --steps 8
```

视频要几分钟，所以**流式报告进度**，报的是运行时自己的阶段和步数：

```text
conditioning (2s)
sampling 0/8 steps 0% (8s)
sampling 3/8 steps 38% (64s)
sampling 7/8 steps 88% (134s)
vae_decode (154s)
encoding (165s)
note     : the runtime produced video/webm, so the file is clip.webm
wrote clip.webm (354.2 KiB, 17 frames)
```

两点值得注意：

- 文件扩展名跟随运行时**实际产出的容器**（sd.cpp 出 webm），不会把 webm 字节写成 `.mp4`；
- `--steps` 默认 8。运行时的默认是 20，会把等待时间拉到三倍。

Ctrl-C 会通知运行时停止（见 7.9）。

### 7.6 音乐与音效

```bash
gguf music --prompt "warm ambient piano with soft rain" --duration 8 --out song.wav
```

实测（Stable Audio 3 Small Music f16，CUDA）：

```text
loading the model (1.0s)
loading weights (1.2s)
encoding the prompt (1.8s)
generating audio (2.0s)
decoding the waveform (2.2s)
wrote song.wav (1.3 MiB, 44100 Hz, 2 channel(s))      # 8 秒音频，1.6 秒生成
```

音效用同一个命令，把 `music_model` 指向 SFX 包即可：

```text
wrote sfx.wav (882044 bytes)     # 5 秒音频，2.8 秒生成
```

进度来自运行时的日志阶段，**没有百分比**——它不报步数，编一个出来只会误导。

### 7.7 语音合成

```bash
gguf speak "The engine can synthesise speech." --out speech.wav
```

两种实现，由配置决定：

| 配置 | 运行时 | 说明 |
| --- | --- | --- |
| `tts_voice` | Piper | 默认；63 MB 音色，瞬间完成，ONNX |
| `tts_model` | audio.cpp（GGUF） | 2.7 GB 模型，约 9 秒，音色更自然 |

### 7.8 语音识别

```bash
gguf transcribe recording.wav --language en
```

whisper.cpp 要求 16 位 PCM WAV。

### 7.9 调度与性能

```bash
gguf plan --model <路径> --context 8192    # 这台机器会怎么排这个模型
gguf memory                                # VRAM/RAM/NVMe 三层预算
gguf benchmark --model <路径> --runs 3     # 每种卸载策略的 tok/s 和显存
```

`plan` 会说明为什么选单卡/多卡/CPU 卸载，以及每张卡预留了多少。

### 7.10 查看模态

```bash
gguf modalities
```

```text
asr                supported    whisper.cpp
image_generation   supported    stable-diffusion.cpp
music              supported    AudioCPP
text               supported    llama.cpp
tts                supported    Piper
video              supported    stable-diffusion.cpp
vision             supported    llama.cpp mtmd
```

答案来自**探测**：装了哪个运行时，才有哪个模态。没装的会给出安装提示。

### 7.11 守护进程

```bash
gguf serve                      # 前台运行
gguf-core --data-dir <DIR>      # 等价，带更多选项
gguf-core --check               # 只做启动检查并输出 JSON，不常驻
```

同一个数据目录只允许一个核心。第二个会以退出码 **3** 结束并说明是谁占着。

---
## 8. HTTP API（OpenAI 兼容）

守护进程默认监听 `127.0.0.1:18888`，只绑回环。

| 端点 | 说明 |
| --- | --- |
| `GET /v1/models` | 列出可用模型 |
| `POST /v1/chat/completions` | 对话，支持流式 |
| `POST /v1/embeddings` | 向量化 |
| `POST /v1/images/generations` | 图像生成，返回 base64 PNG |
| `POST /v1/audio/speech` | 语音合成，返回 `audio/wav` |
| `POST /v1/audio/transcriptions` | 语音识别，multipart 上传 |

```bash
curl http://127.0.0.1:18888/v1/models

curl -X POST http://127.0.0.1:18888/v1/images/generations \
  -H 'Content-Type: application/json' \
  -d '{"prompt":"a red fox in snow","size":"512x512","n":1}'
```

网关**转播核心的流**，不是第二条推理路径（ADR-020）。
不支持的能力返回 503 `capability_not_available` 并说明原因，
而不是假装成功或给一个空响应。

`/v1/audio/speech` 只出 WAV；要 mp3 会被明确拒绝，不会把 WAV 字节贴个 mp3 标签。

---

## 9. gRPC

原生界面用 gRPC，不用 HTTP——它们本来就是本地进程，不需要 REST 那层。

| 服务 | 内容 |
| --- | --- |
| `HealthService` | 健康、组件状态 |
| `UiService` | 界面快照、设置、事件订阅 |
| `InferenceService` | 对话流、embeddings |
| `MediaService` | 图像、视频（流式进度）、音乐（流式）、语音合成、转写 |

传输：Windows 上是命名管道（DACL 限定当前用户 + SYSTEM），
Unix 上是 0700 目录下的 Unix socket。

契约在 `proto/`，Rust 和 C# 都从这里生成，双方不可能对线路格式产生分歧。

---

## 10. 数据目录

```text
<数据根>
├── config.toml
├── models/            模型文件
├── cache/             可回收的缓存（含 downloads/）
├── runtimes/          运行时二进制（用户放置）
├── logs/
└── database/
    ├── engine.db      SQLite：模型登记表、事件、设置
    └── core.lock      单实例锁
```

`models/`、`cache/`、`logs/` 是用户数据，卸载包时**不会删除**；
`cache/` 可以随时清空，引擎会重建。

---

## 11. 测试与门禁

提交前跑这套，和 CI 一致：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace            # 必须早于测试
cargo test --workspace --all-targets
cargo doc --workspace --no-deps

./scripts/generate-csharp-protos.ps1 -Check
cd platforms/windows/GgufAiEngine.Shell.Core.Tests && dotnet test
```

### 需要真实环境的测试

这些默认 `#[ignore]`，需要时显式运行——**不会**在没有条件时假装通过：

| 测试 | 开关 |
| --- | --- |
| 解析真实 GGUF | `GGUF_TEST_MODEL=/path/model.gguf cargo test -p gguf-engine-gguf -- --ignored` |
| 真实对话 | `GGUF_CHAT_TESTS=1` 并装好 llama.cpp |
| Hugging Face 下载 | `GGUF_HUB_TESTS=1` |
| GPU 相关 | 见各 crate 的 `#[ignore]` 说明 |

CI 在 ubuntu / windows / macOS 三平台跑全部非 ignore 测试，
外加 MSRV 检查、`unsafe` 检查、存根漂移检查、Cargo.lock 检查。

---
## 12. 故障排查

下面每条都是本项目开发中**真实遇到过**的问题，不是假设。

### 12.1 模态显示不支持

```bash
gguf modalities      # 先看它怎么说
gguf runtimes        # 再看运行时找没找到
```

提示里会写引擎搜过哪些目录。把运行时所在目录加进 `GGUF_ENGINE_RUNTIME_DIR`
（多个用分号分隔）即可。

### 12.2 扩散服务报连接失败

图像/视频需要一个**独立运行**的 `sd-server`。
`diffusion_url` 留空时用运行时默认端口，但服务必须已经起来：

```bash
curl http://127.0.0.1:1234/v1/models     # 通了才说明服务在
```

### 12.3 生成比预期慢十倍

先看显存是不是被别的进程占着。实测过：一个残留的 `llama-server` 占了 24 GB 卡
中的 21 GB，同一张 512×512 图从 **14.2 秒**变成 **412.9 秒**（29 倍）。

```bash
nvidia-smi --query-gpu=index,memory.used --format=csv
```

这正是 `gpu-manager` 的每卡预算和可回收租约要防的情况——只是从外面看到了。

### 12.4 音乐/语音在 CUDA 上直接崩溃

见 5.4：q8_0 + CUDA 会在 ggml MMQ kernel 里断言失败。
换成 f16 权重，或把 `audio_backend` 设为 `cpu`。

### 12.5 第二个核心起不来

这是设计如此。退出码 3，并告诉你占用者 pid：

```text
gguf-core: another core instance is already running (pid 12943).
Use the UI or `gguf doctor` instead of starting a second daemon.
```

但注意：**不同数据目录**的两个核心可以共存，它们需要不同的网关端口。
端口默认 18888，且配置不接受 0（不是「随便挑一个」的意思）。

### 12.6 改了代码，测试还是失败

端到端测试真的会执行 `target/debug/gguf` 和 `gguf-core`。
如果只跑了 `cargo build -p 某个库`，那两个二进制还是旧的：

```bash
cargo build --workspace     # 先把二进制刷新
cargo test --workspace --all-targets
```

判断信号：UI 或测试在**零秒**就失败，通常不是功能问题，而是跑了旧二进制。

### 12.7 提示词/模型路径报错说找不到

区分文件和目录：

| 配置项 | 期望 |
| --- | --- |
| `image_model`、`asr_model`、`tts_voice` | 文件 |
| `music_model`、`tts_model` | **目录** |

### 12.8 代码改了但 C# 存根检查失败

```powershell
./scripts/generate-csharp-protos.ps1          # 重新生成并提交
```

检查比对的是**归一化后的文本**，不是字节——`.gitattributes` 让 `.cs` 以 CRLF
检出，而 protoc 写 LF，逐字节比会把换行差异误报成漂移。

### 12.9 大模型放 Roaming 目录

已经修正：数据根目录在 Windows 上是 `%LOCALAPPDATA%\GGUF-AI-Engine`。
模型动辄几十 GB，放漫游目录会让每次登录都试图同步它们。

---

## 13. 已实测的数字

全部来自本机（i9-14900KF / 64 GB / RTX 4090 + RTX 3060，驱动 610.88），
同一台机器、同一批权重：

| 任务 | 模型 | 时间 |
| --- | --- | --- |
| 512×512 图像 | SD 1.5 Q8_0 GGUF | 10.5 s |
| 480×320 / 17 帧视频 | Wan 2.1 T2V 1.3B | 78 s（原始 API） |
| 512×320 / 17 帧视频 | 同上 | 96 s（经适配器） |
| 8 秒音乐 | Stable Audio 3 Small f16 | 1.6 s |
| 5 秒音效 | Stable Audio 3 Small SFX f16 | 2.8 s |
| 一句话语音合成 | Piper | < 1 s |
| 一句话语音合成 | Qwen3 TTS CustomVoice（CPU） | 9 s |

内存占用（运行时自报）：

| 组合 | 显存 |
| --- | --- |
| SD 1.5 | 2.0 GB |
| Wan 1.3B + umt5 Q4_K_M + VAE | 6.4 GB |
| Wan 1.3B + umt5 fp8 + VAE | 9.4 GB |

---

## 14. 已知不足

不写这一节，文档就成了宣传。

| 项 | 现状 |
| --- | --- |
| Linux 外壳功能 | 只有对话页，没有图像/视频/音乐页 |
| 音乐/音效的进度粒度 | 只有阶段，没有百分比（运行时不给步数） |
| 取消音乐渲染 | 机制已实现，但音乐渲染太快，未做成端到端实证；视频取消已实测 |
| macOS 外壳 | 未实现 |
| 扩散服务自动启动 | 不支持，需用户自行启动 |
| CI 的 Packaging / Platform shells | 已在 321a4cf 验证通过，但改 `tests/` 不会触发它们 |

---

## 15. 相关文档

| 文档 | 内容 |
| --- | --- |
| [architecture/overview.md](architecture/overview.md) | 分层与数据流 |
| [architecture/multimodal-runtimes.md](architecture/multimodal-runtimes.md) | 多模态运行时的接入方式与实测 |
| [architecture/native-shells.md](architecture/native-shells.md) | 三套原生界面的约定 |
| [architecture/gguf-and-model-hub.md](architecture/gguf-and-model-hub.md) | GGUF 解析与模型中心 |
| [architecture/multi-gpu-and-memory.md](architecture/multi-gpu-and-memory.md) | 多卡规划与内存分层 |
| [architecture/ipc-and-storage.md](architecture/ipc-and-storage.md) | IPC 与存储 |
| [architecture/task-status.md](architecture/task-status.md) | 各任务的完成情况与证据 |
| [adr/](adr/) | 架构决策记录 |
