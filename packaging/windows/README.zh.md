> [English](README.md) | **中文**

# Windows 打包（MSIX）

状态：**已在本机构建并验证**（Windows 11，Windows SDK 10.0.26100）。

## 构建

```powershell
cargo build --release -p gguf-engine-daemon -p gguf-engine-cli -p gguf-engine-runtime-worker
./scripts/package-windows.ps1
```

输出：`target/package/windows/GGUFAIEngine-<version>-x64.msix`。

## 脚本做了什么

1. 从 `Cargo.toml` 读取版本（唯一的真相来源）；
2. 布置 `core/`（三个 Rust 二进制文件）、`shell/`（WinUI 3，自包含）、`runtimes/`（空：运行时在用到时按用户安装）和 `Assets/`；
3. 把版本替换进 `Package.appxmanifest`，并写成 `AppxManifest.xml`（makeappx 不接受 Visual Studio 的那个文件名）；
4. 运行 `makeappx pack`。

## 已验证

```text
makeappx  : ...\10.0.26100.0\x64\makeappx.exe
version   : 0.1.0.0
shell     : published to target/package/stage/shell
package   : target/package/windows/GGUFAIEngine-0.1.0-x64.msix
size      : 69.8 MB
```

解包产出的包会列出 `Assets/`、`core/`（gguf-core.exe、gguf.exe、gguf-worker.exe）、`shell/`、`AppxManifest.xml` 和 `AppxBlockMap.xml`，并且 `makeappx unpack` 报告 "Package extraction succeeded"。

## 该包未签名

安装它需要开发者模式，或者一个由证书签出的签名，且该证书的 subject 与 manifest 中的 `Publisher` 匹配：

```powershell
signtool sign /fd SHA256 /a /f cert.pfx GGUFAIEngine-0.1.0-x64.msix
```

这里故意**不**把签名自动化：一个自己生成证书的构建脚本会产出谁都不该信任的包，而信任决定属于发布它的那个人。

## 能力

只有 `runFullTrust`（外壳把 `gguf-core` 作为子进程启动）和 `internetClient`（Hugging Face 下载）。没有 `broadFileSystemAccess`：应用只在自己的数据目录里写入。
