#!/bin/sh
# On removal, tell the user where their data is instead of deleting it.
#
# Removing a package must never destroy downloaded models: a 40 GB model
# re-download is a real cost, and the user did not ask for it to be deleted.
set -e

if [ "$1" = "remove" ] || [ "$1" = "purge" ]; then
    for home in /home/*; do
        data="$home/.local/share/gguf-ai-engine"
        if [ -d "$data" ]; then
            echo "Note: user data kept at $data (remove it manually if you want it gone)."
        fi
    done
fi
exit 0
