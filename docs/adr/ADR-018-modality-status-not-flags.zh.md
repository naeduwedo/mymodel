> [English](ADR-018-modality-status-not-flags.md) | **中文**

# ADR-018 - 模态以状态和原因声明，绝不用标志位

**状态：** 已接受（Task 8 的实现决策）

## 背景

Task 8 要求图像、视频、音频和音乐适配器。实际上，今天只有其中一个能在这台机器上构建：
llama.cpp 附带 `mtmd`，它通过 `mmproj` 伴随文件提供图入/文出。这个仓库里既没有安装、
也没有计划任何扩散、视频、ASR、TTS 或音乐运行时。

要避免的失效模式是：UI 显示四个置灰的标签页却不给任何解释，或者——更糟——标签页看起来
可点，但稍后失败。

## 决策

`gguf_engine_runtime_manager::modalities()` 是单一事实来源。每个条目携带：

| 字段 | 用途 |
| --- | --- |
| `modality` | 稳定 id：text、vision、image_generation、video、asr、tts、music |
| `status` | `Supported` / `Experimental` / `Unsupported` |
| `runtime` | 由哪个适配器实现它（若有） |
| `detail` | UI 原样显示的句子 |

`Supported` 只用于真正运行过的东西。视觉被标记为支持，是因为一个真实的 27B 模型配合它的
`mmproj`，在这台机器上就一张真实的图片回答了一个真实的问题；其余五个携带 `Unsupported`，
并点明缺失的运行时。

## 多模态 Manifest

一个视觉模型不能仅由它的权重文件完整描述。`pair_companions()` 通过共享的名称 token 把
`mmproj-*.gguf` 文件匹配到某个权重文件，并把 projector 标记为 `required`。有两条规则保证
这件事诚实：

- 名称与权重没有任何共享 token 的 projector 会作为**孤儿**返回，而不是被静默附着；
- 结果经过排序，因此相同的输入总是产生相同的 Manifest。

## 后果

- `gguf modalities` 打印这个矩阵，包括原因。用户问“它能处理音频吗？”会得到一句话，而不是沉默。
- 视觉适配器在启动任何东西之前先校验全部三个文件：一个 20 GB 的模型加载到最后崩溃，是对
  “文件缺失”的糟糕回答。
- 适配器对代价是诚实的：`llama-mtmd-cli` 是一次性的，所以每次请求都会重新加载模型。这一点
  写在 CLI 输出里，而不是被隐藏。

## 验证

    $ gguf modalities
    text             supported    llama.cpp          streaming chat through llama-server
    vision           supported    llama.cpp mtmd     image in, text out; needs an mmproj
    image_generation unsupported  -                  no diffusion runtime adapter
    ...
    2 of 7 modalities are implemented in this build

    $ gguf vision --model Qwen3.8-27B... --mmproj mmproj-Qwen3.8-27B-Q8_0.gguf --image test.png
    The shape in the image is **yellow**. It's a solid yellow circle set against a blue...

对于生成的测试图片（蓝底黄色圆形）而言，这个答案是正确的；而且 projector 配对有 5 个单元测试，
包括孤儿用例。
