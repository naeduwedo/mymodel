> [English](multimodal-runtimes.md) | **中文**

# 非 LLM 运行时：图像、视频、语音与音乐（spec §2.3、§17 Task 8）

> 规范来源：spec §2.3（运行时适配器）、§17 Task 8（“一次加一个 Image、Video、Audio 和 Music
> 适配器，每个都要有真实的兼容性测试”）。

## 每个运行时共有的形状

```text
manifest  -> declares what the runtime is and what it can do (spec §2.3)
probe     -> finds its binary and says where it looked
adapter   -> speaks its protocol
gate      -> a modality is offered only when a probe found a runtime for it
```

前两项以及适配器由 `crates/multimodal` 负责；最后一项由网关和 CLI 负责。

## 集成了什么

| 运行时 | 类型 | 调用方式 | 模态 | 在此验证 |
| --- | --- | --- | --- | --- |
| stable-diffusion.cpp | 扩散 | 通过 HTTP 的 `sd-server` | 图像生成、视频 | **是** |
| whisper.cpp | 语音识别 | 每个请求调用一次 `whisper-cli` | ASR | **是** |
| Piper | 语音合成 | 每个请求调用一次 `piper` | TTS | **是** |
| audio.cpp | 音频生成（ggml） | 每个请求调用一次 `audiocpp_cli` | 音乐、音效 | **是** |

它们每一个都是独立的 C++ 二进制。没有一个依赖 ComfyUI，也没有一个在运行期需要 Python。

## 音乐与音效，跑在 GGUF 权重上

`audio.cpp` 是和 llama.cpp 同一族的 ggml 运行时：它自己加载 GGUF 权重并自己执行。这里用
CUDA 从源码构建，并限定于本项目使用的模型家族。

```text
$ gguf music --prompt 'warm ambient piano with soft rain' --duration 8
runtime  : AudioCPP (...\audiocpp_cli.exe, Cuda)
model    : ...\stable-audio-3-small-music-f16
wrote song.wav (1.3 MiB, 44100 Hz, 2 channel(s))      1.6 s
```

1.6 秒生成 8 秒立体声 44.1 kHz 音频 —— 约为实时的五倍。音效走同一条代码路径，只是换用 SFX 包：
5 秒音频用 2.8 秒。

### 量化与后端的组合很重要

| 权重 | 后端 | 结果 |
| --- | --- | --- |
| q8_0 | cuda | **中止** —— ggml 的 MMQ 内核断言失败（`mmq_x_best=0`） |
| q8_0 | cpu | 正常 |
| f16 | cuda | 正常 |

第一次尝试音乐时，进程在 2.8 秒内因一句 ggml 断言而崩溃。这既不是本项目的 bug，也不是 GPU 坏了：
这个 ggml 构建里的 Q8_0 矩阵乘内核在这个驱动上不接受这个模型。因此运行时把后端暴露为配置项
（`multimodal.audio_backend`），文档也把这一组合写清楚，因为只说“用 CUDA”信息量不够。

## 证据，而不是声称

```text
$ gguf modalities
asr                supported    whisper.cpp            provided by whisper.cpp (...\whisper-cli.exe)
image_generation   supported    stable-diffusion.cpp   provided by stable-diffusion.cpp (...\sd-server.exe)
music              unsupported  -                      no runtime in this build provides music; ...
text               supported    llama.cpp              streaming chat through llama-server
tts                supported    Piper                  provided by Piper (...\piper.exe)
video              supported    stable-diffusion.cpp   provided by stable-diffusion.cpp (...\sd-server.exe)
vision             supported    llama.cpp mtmd         image in, text out; needs an mmproj companion file

6 of 7 modalities are implemented in this build
```

当某个运行时*未*安装时，该行会带上安装提示，因为这是用户能够修复的：

```text
image_generation unsupported  stable-diffusion.cpp  sd-server.exe was not found on PATH or in: ...
                                                   Set GGUF_ENGINE_RUNTIME_DIR to its directory.
```

## 三个已验证的闭环

### 语音识别

让 Windows SAPI 朗读一句话，再让 whisper.cpp 把它读回来。经过两个相互独立的语音工具的一次
往返才是真实的测试；一个固定的 WAV 样本只能证明解析器能用。

```text
$ gguf transcribe %TEMP%\gguf-asr-test.wav
The quick brown fox jumps over the lazy dog.
```

### 语音合成

```text
$ gguf speak 'the engine can speak' --out speech.wav
wrote speech.wav (57.8 KiB)
```

随后 Piper 的输出被 whisper.cpp 转写，回来的是“Speech synthesis is working.” —— 正是喂进去的
那段文本，于是整个 文本 -> 音频 -> 文本 的闭环合上了。

### 图像生成

```text
$ gguf image --prompt 'a red fox in snow' --out fox.png
runtime  : stable-diffusion.cpp (http://127.0.0.1:18902)
request  : 512x512, 20 steps
wrote fox.png (389.3 KiB)
```

运行时在一个 CUDA 13.4 构建上报告 `2035.00MB (VRAM 2035.00MB)`，所以模型是驻留在 RTX 4090 上，
而不是在系统内存里。

## 一次改变了结论的测量

第一次 512x512 渲染花了 **412.9 秒**。这和 4090 的正常表现差了两个数量级，所以它被调查了，
而不是被直接上报。

| 情况 | 512x512 |
| --- | --- |
| 先前测试遗留的一个 `llama-server` 已经占着 21 GB | 412.9 秒 |
| 同一台机器，杀掉它之后 | **14.2 秒** |

`nvidia-smi` 显示，在慢的那次运行中 4090 已用 23.7 GB / 总共 24.5 GB，而在快的那次之前只有
约 4 GB。sd.cpp 会分配一块很大的计算工作区，所以与卡上其他任何东西争抢时，它不是平缓退化 ——
而是直接掉下悬崖。

这正是 `crates/gpu-manager` 里的内存编排器存在所要防止的失败模式（按设备的预算和会过期的
租约），只不过这次是从另一侧观察到的：一个泄漏的工作进程带来 29 倍的变慢，看起来却像是运行时
坏了。

## 视频：已验证

与图像不同，`sd.cpp` 通过**异步**任务 API 生成视频：

```text
POST /sdcpp/v1/vid_gen   -> 202 { id, poll_url }
GET  /sdcpp/v1/jobs/<id> -> { status, result: { b64_json, mime_type, frame_count } }
```

一个片段被渲染了两次：一次走原始 API，一次走适配器。

```text
$ gguf video --prompt 'a lighthouse on a cliff at sunset' --out clip.mp4 --frames 17 --fps 8
runtime  : stable-diffusion.cpp (http://127.0.0.1:18903)
request  : 512x320, 17 frames at 8 fps
note     : a clip takes minutes; polling the runtime
note     : the runtime produced video/webm, so the file is clip.webm
wrote clip.webm (354.2 KiB, 17 frames)
```

| 测量项 | 结果 |
| --- | --- |
| 模型 | Wan 2.1 T2V 1.3B，驻留 9,370 MB（VRAM），FLOW 模式 |
| 片段 | 480x320，17 帧，6 步 |
| 原始 API | 78.4 秒，194 KB webm |
| 经适配器 | 95.7 秒，354 KiB webm，17 帧 |

拒绝路径也验证过了，用的是只加载了纯图像模型的服务器：

```text
gguf: error: this runtime cannot generate video: loaded model does not support vid_gen
```

**为什么选 1.3B 而不是 A14B：** 这里的 A14B 模型需要高噪声和低噪声两个 transformer
（13.3 GB + 13.3 GB）再加一个 6.3 GB 的文本编码器 —— 约 33 GB，而卡只有 24 GB。1.3B 变体加上
各组件是 9.4 GB，还留得出计算工作区的空间，所以它是真正放得下的最大 Wan。

**第一次尝试弄错的一个细节：** 运行时产出的是 `webm`，而命令要求的是 `clip.mp4`。把 webm
字节写进 `.mp4` 名字会得到一个播放器拒绝的文件，因此扩展名现在跟随运行时报告的 MIME 类型，
并且这个改动会被明说，而不是悄悄发生。

## 视频进度

一个片段要花几分钟，所以 `GenerateVideo` 采用**流式**返回，而不是阻塞。运行时本来就报告阶段
和一个步数计数器，转发的就是这些 —— 而不是由已用时间推算出的百分比，那只是个猜测，机器越慢
错得越离谱。

```text
$ gguf video --prompt 'a fox in a snowy forest' --frames 17 --steps 8
conditioning (2s)
sampling 0/8 steps 0% (8s)
sampling 1/8 steps 12% (30s)
...
sampling 7/8 steps 88% (134s)
vae_decode (154s)
encoding (165s)
```

同样的事件也会到达 Windows 外壳，由它驱动一个 `ProgressBar` 和一行状态文本。已用 UI Automation
验证：测试脚本选中 Create 页面、输入 prompt、点击 **Generate clip**，并实时观察文本变化：

```text
[168s] sampling 3/8 steps (170s)
...
[351s] vae_decode (355s)
DONE: Wrote 424 KiB (17 frames) to C:\...\gguf-clip-165929.webm
```

**由此暴露的两个缺陷，都是真实的：**

| 缺陷 | 后果 | 修复 |
| --- | --- | --- |
| 适配器从未发送 `sample_steps` | 运行时用了默认的 20 步，渲染时间变成三倍 | 新增 `VideoRequest::steps`，以 `sample_params.sample_steps` 发送 |
| CLI 每次轮询都重写它的进度行 | 重定向的日志里出现数百行 | 只在阶段或步数变化时才重写 |

第三个由外壳发现：状态文本显示了阶段却没显示步数，导致 UI 的信息量比 CLI 还少。现在两者都显示。

## 端点

| 端点 | 行为 |
| --- | --- |
| `POST /v1/images/generations` | 代理到扩散服务器；`n > 1` 会被拒绝，而不是悄悄减少 |
| `POST /v1/audio/transcriptions` | multipart，正如 OpenAI 客户端发送的那样 |
| `POST /v1/audio/speech` | 返回 `audio/wav`；`mp3` 会被拒绝，而不是贴错标签 |
| `MediaService.GenerateMusic` | gRPC；由各外壳使用，因为它们说的是 gRPC 而不是 HTTP |

运行时缺失时是 **503 `capability_not_available`**，并附带探测给出的原因，而不是 500，
也不是一个空的成功。

## 配置

```toml
[multimodal]
image_model   = 'E:\ComfyUI\models\checkpoints\DreamShaper_8_pruned.safetensors'
diffusion_url = 'http://127.0.0.1:18902'   # a running sd-server
asr_model     = '...\ggml-tiny.bin'
tts_voice     = '...\en_US-amy-low.onnx'
```

二进制在 `PATH` 或 `GGUF_ENGINE_RUNTIME_DIR` 中查找。模型文件没法被猜出来，所以它们是配置项；
留空表示该模态没有配置好，这种情况会被如实报告，而不是等到请求时才失败。

## 为什么扩散服务器是一个独立进程

与 ADR-016 对 llama.cpp 的理由相同：模型运行时内部的崩溃绝不能把核心一起带走，而且运行时拥有
自己的内存。引擎指向用户运行的服务器，而不是每个请求起一个 —— 一个扩散模型加载要几十秒，
那会压倒一张小图的开销。
