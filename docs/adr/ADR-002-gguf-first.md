# ADR-002 — GGUF-first, not GGUF-only

**Status:** Accepted (spec §16, ADR-002)

## Context

The GGUF ecosystem covers LLMs and an increasing share of vision and audio
models, but multimodal projects legitimately depend on auxiliary components
(tokenizers, encoders, vocoders) that are not GGUF.

## Decision

Manage GGUF as the primary format. A model manifest may declare non-GGUF
components, but every such component must be listed explicitly with role,
size and hash.

## Consequences

- Users always know what a model is made of before downloading gigabytes.
- "It has a .gguf file" is never treated as proof that a model is runnable.
- The manifest is the unit of compatibility checking, not the file extension.
