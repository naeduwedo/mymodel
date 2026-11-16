> [English](ADR-011-blocking-sqlite-wrapper.md) | **中文**

# ADR-011 — 通过阻塞式包装器访问 SQLite，而非异步驱动

**状态：** 已接受（Task 2 的实现决策）

## 背景

规格 §10 要求使用 SQLite 并配合版本化迁移。最显而易见的 Rust 选择 `sqlx` 是异步的，
其编译期检查宏在构建时需要一份可用的数据库（或一份提交进仓库的 `.sqlx` 缓存）。
此外，sqlx 0.9 要求 rustc 1.94，高于本项目锁定的工具链。

## 决策

在薄的异步包装器后面使用 `rusqlite`（内置 SQLite，无系统依赖）：

* 一个由互斥锁保护的连接；
* 每条语句都在 `tokio::task::spawn_blocking` 上运行，因此一次缓慢的写入不会
  阻塞异步调度器；
* `journal_mode = WAL`（读方永不阻塞写方）、
  `synchronous = NORMAL`、`foreign_keys = ON`，以及 5 秒的 `busy_timeout`；
* 迁移是纯 `.sql` 文件，用 `include_str!` 内嵌，在同一事务中与其版本行一起应用，
  并记录到 `schema_migrations`。

## 后果

- 模式是真正的 SQL，可以在 diff 中审阅，没有宏生成的查询。
- 由更新版本构建写出的数据库会被**拒绝**（`MigrationError::TooNew`），
  而不是被静默打开并损坏。
- `Database::call` 是 `async fn`，且总是转移到阻塞线程池；`call_blocking` 只用于
  启动检查和测试，并在文档中如此说明。
- 同一时刻只有一个写者。对于一个本地注册表而言这没有问题，它的写入是模型元数据、
  设置和任务记录；如果后续性能分析显示存在争用，连接池是 `Database` 背后的局部改动。
- 不需要构建期数据库，因此在从未运行过本产品的机器上，干净的 `cargo build` 也能工作。
