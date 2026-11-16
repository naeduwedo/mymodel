> **English** | [中文](README.packaged.zh.md)

# GGUF AI Engine (packaged)

Installed files:

    /usr/lib/gguf-ai-engine/gguf-core     core daemon
    /usr/lib/gguf-ai-engine/gguf          command line client
    /usr/lib/gguf-ai-engine/gguf-worker   isolated runtime worker
    /usr/bin/gguf                         PATH wrapper

User data (created on first run, never removed by the package):

    ~/.local/share/gguf-ai-engine/models
    ~/.local/share/gguf-ai-engine/runtimes
    ~/.local/share/gguf-ai-engine/logs
    ~/.local/share/gguf-ai-engine/database
    ~/.config/gguf-ai-engine/config.toml

Runtime binaries (llama.cpp) are installed per-user on demand, not shipped in
this package: they carry their own CUDA/ROCm/Metal dependencies and are updated
independently (spec §15).
