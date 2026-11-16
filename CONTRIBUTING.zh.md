> [English](CONTRIBUTING.md) | **中文**

# 贡献

## 推送之前

跑一遍与 CI 相同的门禁：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```

在 Windows（PowerShell）上，等价的一行命令是：

```powershell
cargo fmt --all; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace --all-targets
```

## 规则

- 整个工作区禁止 `unsafe`（`[workspace.lints.rust]`）。如果某个 crate 确实需要它，先加一个 ADR，并且只在那里改动 lint。
- 不要在没有符合 spec 的理由时添加依赖（§2.2 列出了计划中的那套依赖）。优先使用工作区依赖表，让版本保持一致；`Cargo.lock` 是提交进仓库的。
- 每个新模块都以一段文档注释开头，写明哪些已实现、哪些**没有**。"Planned" 的 crate 会暴露一个 `STATUS` 常量，让这一区分可以被机器检查。
- 公开条目需要文档注释 —— 每个 crate 都设置了 `#![warn(missing_docs)]`。
- 测试就放在代码旁边（`#[cfg(test)]`），除非它们需要真实的二进制文件或跨 crate 的接线；那类测试放到 `tests/tests/`。
- 永远不要对墙上时钟计时或 CI 无法复现的宿主特定值做断言；改为注入或推导它们。

## 添加任务阶段

1. 阅读规范中对应的章节，以及它引用的 ADR。
2. 在改变状态的同一个提交里更新 `docs/architecture/task-status.md`。
3. 诚实说明哪些部分是 mock 测试、哪些是硬件测试（spec §14.2）。

## 提交信息

前缀大致沿用 Conventional Commits 的风格：`feat:`、`fix:`、`refactor:`、`test:`、`docs:`、`chore:`。在正文里引用 spec 章节或任务，例如 `Refs: spec §3.3, Task 1`。
