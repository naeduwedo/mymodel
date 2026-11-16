> [English](README.packaged.md) | **中文**

# GGUF AI Engine（已打包）

安装的文件：

    /usr/lib/gguf-ai-engine/gguf-core     core daemon
    /usr/lib/gguf-ai-engine/gguf          command line client
    /usr/lib/gguf-ai-engine/gguf-worker   isolated runtime worker
    /usr/bin/gguf                         PATH wrapper

用户数据（首次运行时创建，包永远不会删除它）：

    ~/.local/share/gguf-ai-engine/models
    ~/.local/share/gguf-ai-engine/runtimes
    ~/.local/share/gguf-ai-engine/logs
    ~/.local/share/gguf-ai-engine/database
    ~/.config/gguf-ai-engine/config.toml

运行时二进制文件（llama.cpp）按需按用户安装，不随本包分发：它们自带 CUDA/ROCm/Metal 依赖，并独立更新（spec §15）。
