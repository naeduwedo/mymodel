> [English](gguf-and-model-hub.md) | **中文**

# GGUF 解析与模型 Hub（Task 5）

> 规范来源：spec §7（GGUF 解析器、Manifest、Hub、兼容性）。

## 解析器：只读头部，其余一概不信

````text
GGUF file
├── header         4+4+8+8 bytes      magic, version, tensor count, KV count
├── metadata       KV pairs           typed values, duplicate-checked
├── tensor table   descriptors        name, dims, type, offset
└── tensor data    payloads           never read by the parser
```

解析器**只读取元数据区域**（上限 256 MiB）。检查一个 100 GB 的模型只需几兆字节。张量
载荷永远不会被触碰，这正是它安全的原因：

- 不使用 `mmap`，因此文件在映射之后被截断也不会触发 `SIGBUS`（ADR-014）；
- 任何地方都没有 `unsafe`；
- 每次读取都经过一个不会越出缓冲区的游标；
- 文件中得到的每个长度在用于确定任何尺寸之前都会对照 `Limits` 校验。

格式错误的文件会**带着字节偏移被拒绝**，绝不修复，也绝不“尽力解析”。无法识别的张量
类型保留自己的 id（`unknown(27)`）；大端文件会被明确指出是大端；重复的元数据键会在出错
的偏移处报错。

## Manifest 与兼容性：绝不从名字猜测

Manifest 由 GGUF 头部加上调用方提供的仓库事实（`model_id`、`revision`、哈希、许可证）
构建。`general.architecture` 从文件中读取；文件名不起任何作用（spec §7.2）。

兼容性是一个纯函数，只取决于文件声明了什么、本次构建能做什么：

| 架构 | 判定 |
| --- | --- |
| 已知，未安装运行时 | `compatible_unverified`（"no runtime adapter yet (Task 6)"） |
| 已知，已安装运行时 | `compatible_unverified`（"not loaded yet"） |
| 未知 | `unsupported_architecture`，并附上名称 |
| 缺失 | `unknown` |

**下载完整性不属于这个判定。** 下载器负责强制哈希校验；无论某个具体文件是否有公开的
哈希，架构本身都是可以判定的。把两者混为一谈，会让本地文件仅因为不知道仓库哈希就显得
不兼容——这是评审中发现并修复的一个 bug。

## 下载：先校验再就绪

````text
precheck (path containment) → stage .part (resume via Range)
   → hash → atomic rename → register as ready
```

- 没有公开 SHA-256 的文件会被拒绝（`Unverifiable`）；
- 忽略 `Range` 的服务器（返回 200）会触发重新开始，而不是错误的追加；
- 只有通过校验的文件才会以 `ready` 写入注册表。

## 注册表

`crates/database/src/registry.rs` 在一个事务中保存模型、修订和文件。文件行记录自身的
状态（`pending` / `downloading` / `verifying` / `ready` / `failed`）；从更新版本构建读回
的无法识别状态会被当作 `failed`，绝不会当作 `ready`——这是安全的方向。

## 用真实对象验证

````text
$ gguf models pull ggml-org/models --file tinyllamas/stories260K.gguf
repository : ggml-org/models
revision   : 499bc8821c6b12b4e53c5bffcb21ec206f212d81
file       : tinyllamas/stories260K.gguf (1.1 MiB)
verifying  : sha256 270cba1bd5109f42
verified   : sha256 270cba1bd5109f42
registered in the model registry

$ gguf inspect .../stories260K.gguf
  format        : GGUF v3
  architecture  : llama
  quantisation  : F32
  compatibility : compatible_unverified
```

修订在下载前被解析为某个 commit，哈希匹配，注册表记录的是从字节中解析出的真实架构。

## 这个任务不回答的问题

| 问题 | 在哪里回答 |
| --- | --- |
| 这个模型真的能跑吗？ | Task 6（llama.cpp 适配器加载它） |
| 它能装进 VRAM/RAM 吗？ | Task 7（规划器） |
| 令牌存储 / 私有仓库 | 未来（spec §7.3 列出） |
