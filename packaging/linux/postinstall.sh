#!/bin/sh
# Create the per-user data directory layout documented in spec §15.
#
# Deliberately does NOT create anything in /var or run a system service: the
# core is a per-user process, and its data belongs in the user's own
# ~/.local/share/gguf-ai-engine (created on first run by the app itself).
set -e

echo "GGUF AI Engine installed."
echo "Run 'gguf doctor' to check the environment."
echo "Model files live in ~/.local/share/gguf-ai-engine/models by default."
exit 0
