# Non-LLM runtimes: image, video, speech and music (spec §2.3, §17 Task 8)

> Normative source: spec §2.3 (runtime adapters), §17 Task 8 ("add Image, Video,
> Audio and Music adapters one at a time, each with a real compatibility test").

## The shape every runtime has

```text
manifest  -> declares what the runtime is and what it can do (spec §2.3)
probe     -> finds its binary and says where it looked
adapter   -> speaks its protocol
gate      -> a modality is offered only when a probe found a runtime for it
```

`crates/multimodal` owns the first two and the adapters; the gateway and the CLI
own the last.

## What is integrated

| Runtime | Kind | Invocation | Modalities | Verified here |
| --- | --- | --- | --- | --- |
| stable-diffusion.cpp | diffusion | `sd-server` over HTTP | image generation, video | **yes**, see below |
| whisper.cpp | speech recognition | `whisper-cli` once per request | ASR | **yes** |
| Piper | speech synthesis | `piper` once per request | TTS | **yes** |

Nothing else is integrated. Music generation has no adapter, and the modality
report says so rather than omitting the question.

## Evidence, not claims

```text
$ gguf modalities
asr                supported    whisper.cpp            provided by whisper.cpp (...\whisper-cli.exe)
image_generation   supported    stable-diffusion.cpp   provided by stable-diffusion.cpp (...\sd-server.exe)
music              unsupported  -                      no runtime in this build provides music; ...
text               supported    llama.cpp              streaming chat through llama-server
tts                supported    Piper                  provided by Piper (...\piper.exe)
video              supported    stable-diffusion.cpp   provided by stable-diffusion.cpp (...\sd-server.exe)
vision             supported    llama.cpp mtmd         image in, text out; needs an mmproj companion file

6 of 7 modalities are implemented in this build
```

When a runtime is *not* installed the row carries the install hint, because that
is something the user can fix:

```text
image_generation unsupported  stable-diffusion.cpp  sd-server.exe was not found on PATH or in: ...
                                                   Set GGUF_ENGINE_RUNTIME_DIR to its directory.
```

## The three verified loops

### Speech recognition

Windows SAPI was asked to speak a sentence, and whisper.cpp was asked to read it
back. A round trip through two independent speech tools is a real test; a
fixture WAV would only prove the parser works.

```text
$ gguf transcribe %TEMP%\gguf-asr-test.wav
The quick brown fox jumps over the lazy dog.
```

### Speech synthesis

```text
$ gguf speak 'the engine can speak' --out speech.wav
wrote speech.wav (57.8 KiB)
```

Piper's output was then transcribed by whisper.cpp and came back as
"Speech synthesis is working." - the same text that was fed in, so the whole
text -> audio -> text loop is closed.

### Image generation

```text
$ gguf image --prompt 'a red fox in snow' --out fox.png
runtime  : stable-diffusion.cpp (http://127.0.0.1:18902)
request  : 512x512, 20 steps
wrote fox.png (389.3 KiB)
```

The runtime reported `2035.00MB (VRAM 2035.00MB)` on a CUDA 13.4 build, so the
model was resident on the RTX 4090 rather than in system RAM.

## A measurement that changed a conclusion

The first 512x512 render took **412.9 s**. That is two orders of magnitude off
what a 4090 does, so it was investigated instead of reported.

| Condition | 512x512 |
| --- | --- |
| 21 GB already held by a stray `llama-server` from an earlier test | 412.9 s |
| the same machine, after killing it | **14.2 s** |

`nvidia-smi` showed the 4090 at 23.7 GB used / 24.5 GB total during the slow
run, and about 4 GB before the fast one. sd.cpp allocates a large compute
workspace, so contention with anything else on the card does not degrade it
gracefully - it falls off a cliff.

This is the failure mode the memory orchestrator in `crates/gpu-manager` exists
to prevent (per-device budgets and expiring leases), observed from the other
side: a leaked worker cost a 29x slowdown and looked like a broken runtime.

## Video: implemented, happy path unverified

`sd.cpp` generates video through an **asynchronous** job API, unlike images:

```text
POST /sdcpp/v1/vid_gen   -> 202 { id, poll_url }
GET  /sdcpp/v1/jobs/<id> -> { status, result: { b64_json, mime_type, frame_count } }
```

The adapter implements queue + poll + decode, and its refusal path is verified
against a real server:

```text
$ gguf video --prompt 'a fox running'
gguf: error: this runtime cannot generate video: loaded model does not support vid_gen
```

That message comes from the runtime, through the adapter, to the user.

**Not verified:** an actual clip. The available video models here are Wan 2.2
14B (~21 GB of weights plus sd.cpp's workspace) and LTX-2.3 22B, neither of
which fits the 24 GB card alongside the workspace that the image path already
needs. Claiming a rendered clip would be inventing evidence, so it is not
claimed. The command and the adapter are real; the numbers are not.

## Endpoints

| Endpoint | Behaviour |
| --- | --- |
| `POST /v1/images/generations` | proxies to the diffusion server; `n > 1` is refused rather than silently reduced |
| `POST /v1/audio/transcriptions` | multipart, as OpenAI clients send it |
| `POST /v1/audio/speech` | returns `audio/wav`; `mp3` is refused rather than mislabelled |

A missing runtime is **503 `capability_not_available`** with the probe's reason,
not 500 and not an empty success.

## Configuration

```toml
[multimodal]
image_model   = 'E:\ComfyUI\models\checkpoints\DreamShaper_8_pruned.safetensors'
diffusion_url = 'http://127.0.0.1:18902'   # a running sd-server
asr_model     = '...\ggml-tiny.bin'
tts_voice     = '...\en_US-amy-low.onnx'
```

Binaries are found on `PATH` or in `GGUF_ENGINE_RUNTIME_DIR`. Model files cannot
be guessed, so they are configuration; an empty entry means the modality is not
set up, which is reported as such rather than failing at request time.

## Why the diffusion server is a separate process

Same reasoning as ADR-016 for llama.cpp: a crash inside the model runtime must
not take the core with it, and the runtime owns its memory. The engine points at
a server the user runs, rather than spawning one per request - a diffusion model
takes tens of seconds to load, which would dominate a small image.
