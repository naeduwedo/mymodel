> [English](ADR-010-unsafe-windows-pipe-acl.md) | **中文**

# ADR-010 — 为 Windows 管道 DACL 开一个狭窄的 `unsafe` 例外

**状态：** 已接受（Task 2 的实现决策）

## 上下文

工作区禁止 `unsafe`（`[workspace.lints.rust] unsafe_code = "forbid"`）。这条规则值得保留：本产品几乎全部都是普通的安全 Rust，而调度器或解析器里的一个 `unsafe` 块就是缺陷制造机。

Task 2 需要一个只有属主用户才能访问的本地 IPC 传输。在 Windows 上，正确的机制是一个命名管道，用只授予当前用户和 SYSTEM、不授予其他任何人的安全描述符来创建。Tokio 用于创建管道的安全 API 会传入 **null** 的 `SECURITY_ATTRIBUTES`，这意味着管道继承进程默认 DACL。在典型的桌面上，该 DACL 包含本地的 `Users` 组，于是每个本地账户都能加载模型并查询 API。唯一接受描述符的 tokio 入口是 `create_with_security_attributes_raw`，而它是 `unsafe` 的。

构建该描述符所需的 Win32 调用（`OpenProcessToken`、`GetTokenInformation`、`ConvertStringSecurityDescriptorToSecurityDescriptorW`）本质上也是不安全的。

## 决策

只允许在两个模块中使用 `unsafe`，二者都是仅限 Windows，且都位于 `gguf-engine-security`：

| 模块 | 原因 |
| --- | --- |
| `windows_acl` | 解析当前用户的 SID，并由 SDDL 构建 `SECURITY_ATTRIBUTES`。 |
| `windows_pipe` | 用该描述符调用 `ServerOptions::create_with_security_attributes_raw`。 |

其余一切都保持禁止：

* 该 crate 在其根部重新建立 `#![deny(unsafe_code)]`；
* 每个模块带一个作用域限定的 `#[allow(unsafe_code)]`；
* 每个块都有 `// SAFETY:` 注释，指明它依赖的不变量；
* CI 任务 `deny-unsafe` 会在任何*其他* crate 重新启用该 lint 时让构建失败，而本 ADR 是唯一被认可的这么做理由。

替代方案 —— 信任进程默认 DACL —— 已被否决：一个静默地对全世界可读的控制套接字，正是那种在评审中不可见、在实践中可被利用的缺陷。

## 后果

- 暴露给工作区其余部分的安全接口是 `windows_pipe::create(name) -> PipeInstance` 和 `pipe_acl_sddl()`；调用方永远见不到裸指针。
- 评审者必须仔细读两个小文件，而不是为指针运算审计整个代码库。
- 如果 tokio 某天为安全描述符提供了安全 API，这两个模块就应删除，本 ADR 则标记为 `Superseded`。
- 在 Unix 上完全没有例外：套接字位于一个 `0700` 目录中，无需任何 FFI。

## 验证

- `windows_acl` 的测试断言生成的 SDDL 授予 SYSTEM 和当前用户，且没有针对 Everyone/Authenticated Users 的 ACE，并断言操作系统解析器接受它。
- `windows_pipe` 的测试创建一个真实管道并连接一个客户端，证明 DACL 不会把其自身的创建者锁在外面。
- `tests/tests/ipc_e2e.rs` 启动真实的守护进程，并通过真实管道与它通信，因此一个损坏的描述符会表现为失败的端到端测试，而不是一条评审意见。
