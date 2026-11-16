> [English](ipc-and-storage.md) | **中文**

# 架构 — IPC、数据库与 Task 2 核心

> 规范来源：[`GGUF-AI-Engine-Engineering-Spec.md`](../../GGUF-AI-Engine-Engineering-Spec.md) §3、§9–§10。
> 本文档说明代码如何实现 §3.2（通信）、§3.3（生命周期）和 §10（数据库），以及哪些
> 内容是刻意尚未构建的。

## 传输（spec §3.2）

```text
CLI / native UI
   │  gRPC over http/2
   ▼
┌────────────────────────────── gguf-core ─────────────────────────────┐
│ Windows: \\.\pipe\gguf-ai-engine-<hash>   DACL = user + SYSTEM       │
│ Unix:    <data_root>/run/core.sock          directory mode = 0700    │
│                                                                      │
│  HealthService   GetHealth / Ping                                    │
│  EventService    Subscribe(after_seq) → stream (replay, then live)   │
└──────────────────────────────────────────────────────────────────────┘
```

为什么不用 TCP：即使防火墙配置错误，本地 socket 也无法从另一台机器访问；它的访问控制
由 OS 实施，而不是靠核心必须分发的令牌。

### Windows 细节

- 一个管道实例只接受一个客户端，因此服务端**始终保留一个空闲实例在监听**。如果等到
  客户端连接之后才创建下一个实例，就会出现一个时间窗：客户端拿到
  `ERROR_FILE_NOT_FOUND`——这与“核心没有运行”无法区分。
- 安全描述符只授予当前用户和 SYSTEM，其他一概不给（ADR-010）。这需要一处限定范围的
  `unsafe` 例外；除此之外，全工作区的 `unsafe` 禁令依然有效。
- tonic 没有管道支持，因此 `crates/ipc/src/transport_windows.rs` 提供了 `PipeStream`
  （服务端，实现 tonic 的 `Connected`）和 `PipeConnector`（客户端，是一个
  `tower::Service<Uri>`）。

### Unix 细节

- socket 位于 `<data_root>/run/`，以 `0700` 创建，因此其他本地用户无法通过路径连接。
- 崩溃核心留下的失效 socket 会在绑定时被删除；单实例锁保证没有仍在运行的核心占用该
  路径。

### 端点命名

`gguf_engine_protocol::ipc_endpoint(data_root)` 从数据根推导端点：管道名使用稳定的
FNV-1a 标记（管道名限制为 256 个字符），或者在数据根内使用 `run/core.sock`。两套安装
永远不会冲突，测试套件也不会打扰开发者正在运行的核心。

## 服务

| 服务 | 方法 | 状态 |
| --- | --- | --- |
| `HealthService` | `GetHealth`、`Ping` | 已实现 |
| `EventService` | `Subscribe`（服务端流式） | 已实现 |
| `HardwareService` | `GetHardware` | 仅 schema → `UNIMPLEMENTED` |
| `ModelService`、`RuntimeService`、`InferenceService`、`TaskService`、`GpuService`、`MemoryService` | — | 仅 schema |

只有已实现的服务才会注册到路由器。客户端调用未实现的服务会得到 gRPC `UNIMPLEMENTED`，
这是诚实的回答，并且有测试（`crates/ipc/tests/services.rs`）对此做了断言。

### 健康应答

健康报告刻意把事实与缺口混在一起呈现：

```text
state: ready
components:
  ipc                   healthy
  database              healthy   "schema v1, sqlite 3.50.2"
  event-bus             healthy
  hardware_inventory    unhealthy "Task 4: no CPU/GPU probe is implemented yet"
  runtime_llama_cpp     unhealthy "Task 6: no runtime adapter is installed"
  ...
```

`healthy`（聚合标志）只有当状态为 `ready`/`degraded` **并且**每个组件都健康时才为 true。
因此，只读聚合标志的 UI 不会把未完成的子系统误认为可用的子系统。

### 事件流语义

- `Subscribe(after_seq)` 先重放该数据根的有界环，然后继续实时推送；重复事件按序列号
  过滤。
- 慢速客户端会丢事件，而不是让核心内存增长；丢失会被计数（`BusStats.dropped`、
  `EventSubscriber::lagged`）。
- 状态是权威的，事件是增量：错过事件的客户端可以重新读取 `GetHealth`。

## 生命周期（spec §3.3）

```text
Stopped ──▶ Starting ──▶ Ready ⇄ Degraded ──▶ ShuttingDown ──▶ Stopped
```

守护进程的启动顺序经过安排，使部分失败不会留下任何未释放的东西：

1. 解析数据根，加载并校验 `config.toml`；
2. 创建目录树；
3. 获取单实例锁（若已有核心在运行，退出码为 3）；
4. 打开并迁移 SQLite；
5. 绑定 IPC 端点（若无法绑定，退出码为 4）；
6. 在绑定之前 `Stopped → Starting`，然后 `→ Ready` 并发布这次变化。

关闭过程与之相反：`ShuttingDown` → 停止监听器（有 5 秒宽限期，之后中止）→ `Stopped`
→ 释放锁。

宽限期很重要：流式客户端可以把一个请求永远保持打开，而忘记断开连接的 UI 绝不能把核心
拖住不放。

## 数据库（spec §10）

`migrations/0001_init.sql` 创建了规范列出的全部 19 张表。由测试强制执行的约定：

- 容量是 `INTEGER` 字节——`REAL` 字节列会让
  `capacity_columns_are_integers_not_floats` 失败；
- 时间戳是 `INTEGER` 的 UTC 纪元毫秒；
- 使用 `STRICT` 表，因此类型错误会报错，而不是静默转换；
- 外键被声明**并被强制执行**（`PRAGMA foreign_keys = ON`）。

迁移只能向前，记录在 `schema_migrations` 中，并与各自的版本行在同一个事务内应用。由更新
版本构建写入的数据库会被拒绝（`MigrationError::TooNew`），而不是被打开并损坏。

访问模型（ADR-011）：一个 `rusqlite` 连接放在互斥锁后面，每条语句都在 `spawn_blocking`
上执行，使用 WAL + `synchronous=NORMAL` + `busy_timeout`。

## 安全默认值（spec §9.3）

| 关注点 | 实现 |
| --- | --- |
| 仅本地 API | 完全没有 TCP 监听器；IPC 传输是管道/socket |
| 同侪隔离 | 管道 DACL（Windows）/ `0700` socket 目录（Unix） |
| 令牌比较 | 恒定时间的 `gguf_engine_security::Token`，`Debug` 输出被脱敏 |
| 路径包含 | `AppPaths` 拒绝任何解析到数据根之外的路径 |
| 密钥日志 | token 绝不会被格式化进日志；`Debug` 只打印长度 |
| 不可信的仓库内容 | 来自模型仓库的内容一律不执行；目前还没有下载器 |

## 这一层的测试策略

| 层 | 测试 | 说明 |
| --- | --- | --- |
| schema | `crates/database/src/migrations.rs` | 表清单、列类型、升级路径与拒绝路径 |
| 服务 | `crates/ipc/tests/services.rs` | 回环 TCP 上的真实 gRPC，可跨 CI 移植 |
| 传输 | `tests/tests/ipc_e2e.rs` | **真实二进制**跑在**真实管道/socket** 上 |
| 诚实性 | `features()` + `UNIMPLEMENTED` 断言 | 服务不能在其存在之前被宣传 |
| 文档 | `tests/tests/docs_consistency.rs` | 示例配置和 crate 状态保持同步 |

回环 TCP 路径只为测试和未来的远程工作进程而存在；它绑定 `127.0.0.1`，守护进程从不使用
它。
