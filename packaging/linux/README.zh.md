> [English](README.md) | **中文**

# Linux 打包（deb / rpm / AppImage）—— **仅有配方，未构建**

状态：**已构建、已安装并运行**（WSL 下的 Ubuntu 26.04，nfpm，Rust 1.100）。

```text
$ bash packaging/linux/build-packages.sh
version: 0.1.0
created package: dist/gguf-ai-engine_0.1.0_amd64.deb
created package: dist/gguf-ai-engine-0.1.0-1.x86_64.rpm
== verifying the .deb file list
   all expected files present
== PASS
```

安装那个 .deb 并运行安装后的二进制文件：

```text
$ dpkg -i dist/gguf-ai-engine_0.1.0_amd64.deb
$ gguf version --json
{ "cli": "0.1.0", "core": "0.1.0", "protocol": 2, "os": "linux", "arch": "x86_64" }

$ gguf doctor --data-dir /tmp/gguf-inst-test
ipc  : /tmp/gguf-inst-test/run/core.sock
[ok  ] database        schema v2 (expected v2), sqlite 3.50.2
[ok  ] core lifecycle  2 transitions, ready
```

卸载该包后 `~/.local/share/gguf-ai-engine` 保持完好（检查方式是先在那里写一个文件，之后再确认它还在）。

## 这里有什么

| 文件 | 用途 |
| --- | --- |
| `nfpm.yaml` | .deb 与 .rpm 共用的一份定义（避免两者各自漂移） |
| `gguf` | `/usr/bin/gguf` 包装脚本，exec 位于 /usr/lib 下的真正二进制文件 |
| `gguf-ai-engine.desktop` | 桌面入口；启动已打包的 GTK4 外壳 |
| `postinstall.sh` | 打印数据将存放在哪里；不在系统范围内创建任何东西 |
| `preremove.sh` | 卸载时告诉用户他们的模型在哪里，而不是把它们删掉 |
| `README.packaged.md` | 作为该包的文档被安装 |
| `build-packages.sh` | 构建二进制文件、GTK4 外壳、两个包，并做自我验证 |

## 构建（在 Linux 机器上）

```bash
cargo build --release -p gguf-engine-daemon -p gguf-engine-cli -p gguf-engine-runtime-worker
nfpm package --config packaging/linux/nfpm.yaml --packager deb --target dist/
nfpm package --config packaging/linux/nfpm.yaml --packager rpm --target dist/
```

## 有意的决定

- **不设系统服务。** 核心守护进程是一个每用户的进程，配一个每用户的数据目录；装一个 root 服务会让每个本地用户都能访问其他所有用户的模型和 GPU。
- **不随包分发运行时。** llama.cpp 二进制文件自带 CUDA/ROCm 依赖，并独立更新（spec §15）。把它们打进包里，会让运行时冻结在包的发布时刻。
- **卸载保留用户数据。** 重新下载一个 40 GB 的模型是实实在在的成本，而卸载一个包并不是要求删除模型。
- **AppImage 尚未定义。** 它需要一个包含 GTK4 的自包含包，而这依赖于 Task 3 中仍未验证的 Linux 外壳。

## CI 任务证明了什么

该工作流断言以下全部四项，任何一项出现回归都会让构建失败：

1. `nfpm` 无错误地构建两种格式。
2. `dpkg -c` / `rpm -qlp` 列出三个二进制文件、PATH 包装脚本和桌面入口。
3. 包能安装，并且 `gguf version --json` 能运行。
4. 卸载后 `~/.local/share/gguf-ai-engine` 保持完好 —— 测试先往那里写一个文件，之后再检查它。
## 外壳是包的一部分

`build-packages.sh` 构建 `platforms/linux`，并把结果安装到 `/usr/lib/gguf-ai-engine/gguf-ai-engine-ui`，这正是桌面入口 `Exec=` 所指的那个名字。该包声明 `Depends: libgtk-4-1`，并链接系统 GTK，而不是自带一份。

验证方式是安装该 .deb 并在 Xvfb 下启动那个路径：

```text
== desktop entry target ==
-rwxr-xr-x 4194024 /usr/lib/gguf-ai-engine/gguf-ai-engine-ui
Exec=/usr/lib/gguf-ai-engine/gguf-ai-engine-ui
== starting the core ==
   core ready
== launching the packaged UI (as the desktop entry would) ==
   packaged UI stayed alive for 8s
   no critical errors
== PASS
```
