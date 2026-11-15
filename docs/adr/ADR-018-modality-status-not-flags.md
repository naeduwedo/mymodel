# ADR-018 - Modalities are declared with a status and a reason, never a flag

**Status:** Accepted (implementation decision for Task 8)

## Context

Task 8 asks for image, video, audio and music adapters. In practice only one of
them can be built on this machine today: llama.cpp ships `mtmd`, which gives
image-in/text-out through an `mmproj` companion. There is no diffusion, video,
ASR, TTS or music runtime installed or planned in this repository.

The failure mode to avoid is a UI that shows four greyed-out tabs with no
explanation, or - worse - tabs that look clickable and fail later.

## Decision

`gguf_engine_runtime_manager::modalities()` is the single source of truth. Each
entry carries:

| Field | Purpose |
| --- | --- |
| `modality` | stable id: text, vision, image_generation, video, asr, tts, music |
| `status` | `Supported` / `Experimental` / `Unsupported` |
| `runtime` | which adapter implements it, when one does |
| `detail` | the sentence the UI shows verbatim |

`Supported` is only used for what has actually been run. Vision is marked
supported because a real 27B model with its `mmproj` answered a real question
about a real image on this machine; the other five carry `Unsupported` and name
the missing runtime.

## Multimodal manifests

A vision model is not fully described by its weights file. `pair_companions()`
matches `mmproj-*.gguf` files to a weight file by shared name tokens, and marks
the projector `required`. Two rules keep this honest:

- a projector whose name shares no token with the weights is returned as an
  **orphan**, not silently attached;
- the result is sorted, so the same inputs always produce the same manifest.

## Consequences

- `gguf modalities` prints the matrix, including the reasons. A user asking
  "can it do audio?" gets a sentence, not silence.
- The vision adapter validates all three files before spawning anything: a 20 GB
  model load that ends in a crash is a bad answer to a missing file.
- The adapter is honest about cost: `llama-mtmd-cli` is one-shot, so every
  request reloads the model. That is stated in the CLI output rather than hidden.

## Verification

    $ gguf modalities
    text             supported    llama.cpp          streaming chat through llama-server
    vision           supported    llama.cpp mtmd     image in, text out; needs an mmproj
    image_generation unsupported  -                  no diffusion runtime adapter
    ...
    2 of 7 modalities are implemented in this build

    $ gguf vision --model Qwen3.8-27B... --mmproj mmproj-Qwen3.8-27B-Q8_0.gguf --image test.png
    The shape in the image is **yellow**. It's a solid yellow circle set against a blue...

The answer is correct for the generated test image (yellow circle on blue), and
the projector pairing has 5 unit tests including the orphan case.
