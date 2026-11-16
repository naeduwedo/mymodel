> [English](ADR-020-openai-gateway-relays-the-core-stream.md) | **中文**

# ADR-020 - OpenAI 网关转发核心的流，而不是自己加载模型

**状态：** 已接受（`http_api` 后续工作的实现决策）

## 背景

规格 §9.2 要求一个本地 REST / OpenAI 兼容 API。最显而易见的实现是在网关内部再放一条推理
路径：它已经有模型路径，因此可以自己加载并流式输出。

那将意味着有两个地方解析模型 id、两个地方决定 GPU 层数、两个地方处理工作进程崩溃——而且
它们会漂移。UI 和 CLI 已经在与核心对话；第三条路径会成为没人测试的那一条。

## 决策

网关是**协议翻译器**，不是推理引擎：

    OpenAI JSON -> gRPC ChatRequest -> InferenceService -> token stream -> SSE

后果：

- 只有一份推理实现，而且它已经由 UI 和 CLI 实际使用；
- 网关继承模型解析、卸载和并发规则，而不重新实现其中任何一项；
- 网关的 bug 只可能是翻译 bug，而这正是测试成本最低的部分（15 个测试，其中若干跑在真实的
  回环套接字上）。

### 流式是唯一模式

非流式响应是通过排空同一条流产生的。两条代码路径会让 `stream: true` 和 `stream: false`
对模型说了什么产生分歧，而这类 bug 会侵蚀对整个 API 的信任。

### 安全默认值是强制执行的，不是写进文档的

| 规则 | 强制位置 |
| --- | --- |
| 默认绑定回环地址 | `ApiConfig::default()` |
| 非回环绑定且无 token 会被拒绝 | `ApiConfig::validate()`，在套接字打开之前 |
| Token 以常量时间比较 | `authorise()` |
| 除非显式选择，否则不加 CORS 头 | `apply_cors()` 提前返回 |
| 401 会说明如何认证 | `WWW-Authenticate: Bearer` |

一个可被任意网页访问的本地模型端点，是一个混淆代理漏洞，所以默认是沉默，而不是
`Access-Control-Allow-Origin: *`。

### 诚实的计量

`prompt_tokens` 被报告为 0，因为运行时不统计提示词 token。报告一个猜测会让 `usage`
看起来权威，实际却是错的。

## 验证

    $ curl 127.0.0.1:18888/v1/models
    {"object":"list","data":[{"id":"ggml-org/models","object":"model","owned_by":"llama"}]}

    $ curl 127.0.0.1:18888/v1/chat/completions -d '{...}'
    {"object":"chat.completion","choices":[{"message":{"role":"assistant",
      "content":" Anna, Anna, I'm "},"finish_reason":"stop"}],
     "usage":{"completion_tokens":16,"total_tokens":16}}

    unknown model -> 404 {"error":{"code":"model_not_found"}}
