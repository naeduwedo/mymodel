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
| stable-diffusion.cpp | diffusion | `sd-server` over HTTP | image generation, video | **yes** |
| whisper.cpp | speech recognition | `whisper-cli` once per request | ASR | **yes** |
| Piper | speech synthesis | `piper` once per request | TTS | **yes** |
| audio.cpp | audio generation (ggml) | `audiocpp_cli` once per request | music, sound effects | **yes** |

Every one of them is a standalone C++ binary. None of them is ComfyUI, and none of
them needs Python at run time.

## Music and sound, on GGUF weights

`audio.cpp` is a ggml runtime in the same family as llama.cpp: it loads GGUF
weights and runs them itself. Built from source here with CUDA, restricted to the
model families this project uses.

```text
$ gguf music --prompt 'warm ambient piano with soft rain' --duration 8
runtime  : AudioCPP (...\audiocpp_cli.exe, Cuda)
model    : ...\stable-audio-3-small-music-f16
wrote song.wav (1.3 MiB, 44100 Hz, 2 channel(s))      1.6 s
```

Eight seconds of stereo 44.1 kHz audio in 1.6 s - about five times real time.
Sound effects use the same code path with the SFX package: 5 s in 2.8 s.

### The quantization/backend pair matters

| Weights | Backend | Result |
| --- | --- | --- |
| q8_0 | cuda | **aborts** - ggml's MMQ kernel asserts (`mmq_x_best=0`) |
| q8_0 | cpu | works |
| f16 | cuda | works |

The first attempt at music crashed the process in 2.8 s with a ggml assertion. It
is not a bug in this project and not a broken GPU: the Q8_0 matmul kernel in this
ggml build does not accept this model on this driver. The runtime therefore
exposes the backend as configuration (`multimodal.audio_backend`) and the docs
name the pair, because "use CUDA" alone is not enough information.

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

## Video: verified

`sd.cpp` generates video through an **asynchronous** job API, unlike images:

```text
POST /sdcpp/v1/vid_gen   -> 202 { id, poll_url }
GET  /sdcpp/v1/jobs/<id> -> { status, result: { b64_json, mime_type, frame_count } }
```

A clip was rendered, twice: once through the raw API and once through the adapter.

```text
$ gguf video --prompt 'a lighthouse on a cliff at sunset' --out clip.mp4 --frames 17 --fps 8
runtime  : stable-diffusion.cpp (http://127.0.0.1:18903)
request  : 512x320, 17 frames at 8 fps
note     : a clip takes minutes; polling the runtime
note     : the runtime produced video/webm, so the file is clip.webm
wrote clip.webm (354.2 KiB, 17 frames)
```

| Measurement | Result |
| --- | --- |
| Model | Wan 2.1 T2V 1.3B, 9,370 MB resident (VRAM), FLOW mode |
| Clip | 480x320, 17 frames, 6 steps |
| Raw API | 78.4 s, 194 KB webm |
| Through the adapter | 95.7 s, 354 KiB webm, 17 frames |

The refusal path is verified too, against a server holding an image-only model:

```text
gguf: error: this runtime cannot generate video: loaded model does not support vid_gen
```

**Why 1.3B and not A14B:** the A14B models here need both high-noise and low-noise
transformers (13.3 GB + 13.3 GB) plus a 6.3 GB text encoder - about 33 GB against
a 24 GB card. The 1.3B variant plus components is 9.4 GB and leaves room for the
compute workspace, so it is the largest Wan that actually fits.

**A detail the first attempt got wrong:** the runtime produces `webm`, but the
command asked for `clip.mp4`. Writing webm bytes to a `.mp4` name produces a file
players refuse, so the extension now follows the MIME type the runtime reported,
and the change is announced rather than silent.

## Video progress

A clip takes minutes, so `GenerateVideo` **streams** instead of blocking. The
runtime already reports a stage and a step counter, and that is what is relayed -
not a percentage derived from elapsed time, which would be a guess that gets more
wrong the slower the machine is.

```text
$ gguf video --prompt 'a fox in a snowy forest' --frames 17 --steps 8
conditioning (2s)
sampling 0/8 steps 0% (8s)
sampling 1/8 steps 12% (30s)
...
sampling 7/8 steps 88% (134s)
vae_decode (154s)
encoding (165s)
```

The same events reach the Windows shell, which drives a `ProgressBar` and a status
line. Verified by UI Automation: the harness selected the Create page, typed the
prompt, clicked **Generate clip**, and watched the text change live:

```text
[168s] sampling 3/8 steps (170s)
...
[351s] vae_decode (355s)
DONE: Wrote 424 KiB (17 frames) to C:\...\gguf-clip-165929.webm
```

**Two defects this surfaced, both real:**

| Defect | Consequence | Fix |
| --- | --- | --- |
| the adapter never sent `sample_steps` | the runtime used its default 20 steps, tripling the render | `VideoRequest::steps`, sent as `sample_params.sample_steps` |
| the CLI rewrote its progress line every poll | hundreds of lines in a redirected log | rewrite only when the stage or step count changes |

A third, caught by the shell: the status text showed the stage but not the step
count, so the UI was less informative than the CLI. It now shows both.

## Endpoints

| Endpoint | Behaviour |
| --- | --- |
| `POST /v1/images/generations` | proxies to the diffusion server; `n > 1` is refused rather than silently reduced |
| `POST /v1/audio/transcriptions` | multipart, as OpenAI clients send it |
| `POST /v1/audio/speech` | returns `audio/wav`; `mp3` is refused rather than mislabelled |
| `MediaService.GenerateMusic` | gRPC; used by the shells, since they speak gRPC and not HTTP |

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