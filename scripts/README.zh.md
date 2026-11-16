> [English](README.md) | **中文**

# scripts/

开发者辅助脚本。保持它们小而具备跨平台意识；CI 直接调用 `cargo`，所以这些只是便利工具。

- `check.ps1` —— Windows：fmt + clippy + tests，与 CI 相同的门禁。
- `check.sh` —— Linux/macOS 的等价物。
