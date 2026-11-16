> [English](README.md) | **中文**

# Linux 外壳（GTK4）—— 已构建并验证

一个真正的 GTK4/gtk4-rs 外壳，而不是占位物：它构建一个带堆栈切换器（Overview / Settings）的 `ApplicationWindow`，并通过同一个 `gguf_engine_ipc` 客户端，渲染与 Windows 外壳相同的 `UiSnapshot`。

## 状态

| 检查项 | 状态 |
| --- | --- |
| 可编译 | **是** —— GTK 4.22.4，Ubuntu 26.04，Rust 1.100 |
| 可无头运行 | **是** —— 在 Xvfb 下跑 `verify.sh`，对着真实的核心守护进程 |
| Overview + Settings 页面 | 可用 |
| 聊天页面 | **已实现** —— 模型 id、提示词、对话记录、Send/Stop/Clear |

验证输出：

```text
== starting the core
   core ready
== launching the shell under Xvfb for 8 seconds
   shell stayed alive and was stopped by the timeout
== errors logged by the shell
   none
== PASS
```

## 构建

```bash
sudo apt-get install -y libgtk-4-dev build-essential pkg-config xvfb   # Debian/Ubuntu
sudo dnf install -y gtk4-devel gcc pkgconf-pkg-config xorg-x11-server-Xvfb   # Fedora

bash platforms/linux/build.sh        # build
bash platforms/linux/verify.sh       # build + prove it runs
```

`build.sh` 在调用 cargo 之前检查 GTK 4，并默认把 target 目录放在 Linux 文件系统上：在一棵位于 Windows 9p 挂载上的 Rust 目录树里构建，会慢上好几倍。

`verify.sh` 兑现的是那个主张，而不只是构建。它启动一个真实的核心守护进程，等 "core ready"，在 `xvfb-run` 下启动外壳，并且除非窗口存活八秒（`timeout` 返回 124）且没有记录严重错误，否则就判定失败。一个启动后立刻死掉的 GUI 会以 0 或 1 退出，因此过不了这项检查。

## 在 Linux 上构建它实际发现了什么

两个只在 Windows 上跑的门禁永远看不到的真实缺陷：

1. **`crates/ipc/src/client.rs`：`#[cfg(unix)]` 连接器编译不过。** `service_fn` 闭包捕获了一个借用的 `&str`，而 `connect_with_connector` 要求 `'static`。修法是先把路径变成自己拥有的值。
2. **`glib` 被用到了，却从未声明。** `gtk4` 重新导出了 glib，但没让它可被指名，所以 `MainContext::spawn_local` 无法解析。

用一句话说清「在你声称支持的平台上构建」这个论点：在 Windows 上跑 `cargo check`，它说这两个文件都没问题。

## 聊天

聊天页面镜像了 Windows 上的那个：模型 id、提示词、对话记录，以及 Send / Stop / Clear。有两处行为是有意为之：

- **Stop 保留已生成的部分回答。** 用户看着那些 token 一个个到来；把它们丢掉会是个意外。中止任务同时也会断掉流，而正是这一点告诉核心守护进程去卸载模型。
- **空的提示词永远不会送到核心守护进程。** 否则它会为了回答空无而加载一个模型。

这些规则位于 `src/chat.rs` 中，不依赖 GTK，因此无需显示器就能测试（在本目录下跑 `cargo test`：6 个测试）。

## 未实现

| 能力 | 原因 |
| --- | --- |
| 系统提示词字段 | `Transcript::set_system` 存在且已测试，但还没有控件去设置它 |
| 模型选择器 | 模型 id 是手动输入的。选择器需要注册表列表，而 Overview 页面并不请求它 |
## 数据根

`$GGUF_AI_ENGINE_DATA_DIR`，否则用 `~/.local/share/gguf-ai-engine`（spec §15）。外壳绝不会臆造一个核心守护进程不会使用的路径。
