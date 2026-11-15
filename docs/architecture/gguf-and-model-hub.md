# GGUF parsing and the model Hub (Task 5)

> Normative source: spec §7 (GGUF parser, manifest, Hub, compatibility).

## The parser: read the front, trust nothing else

````text
GGUF file
├── header         4+4+8+8 bytes      magic, version, tensor count, KV count
├── metadata       KV pairs           typed values, duplicate-checked
├── tensor table   descriptors        name, dims, type, offset
└── tensor data    payloads           never read by the parser
```

The parser reads **only the metadata region** (bounded at 256 MiB). A 100 GB
model costs a few megabytes to inspect. Tensor payloads are never touched, which
is what makes this safe:

- no `mmap`, so a file truncated after mapping cannot raise `SIGBUS` (ADR-014);
- no `unsafe` anywhere;
- every read goes through a cursor that cannot leave the buffer;
- every length from the file is checked against `Limits` before it sizes anything.

A malformed file is **rejected with a byte offset**, never repaired, never
"best-effort parsed". An unrecognised tensor type keeps its id (`unknown(27)`); a
big-endian file is named as such; a duplicate metadata key is an error at the
offending offset.

## Manifest and compatibility: never guess from the name

The manifest is built from the GGUF header plus repository facts the caller
provides (`model_id`, `revision`, hash, license). `general.architecture` is read
from the file; the file name contributes nothing (spec §7.2).

Compatibility is a pure function of what the file declares and what this build
can do:

| Architecture | Verdict |
| --- | --- |
| known, no runtime installed | `compatible_unverified` ("no runtime adapter yet (Task 6)") |
| known, runtime installed | `compatible_unverified` ("not loaded yet") |
| unknown | `unsupported_architecture` with the name |
| absent | `unknown` |

**Download integrity is not part of this verdict.** The downloader enforces
hashes; the architecture is judgeable whether or not a particular file has a
published hash. Conflating the two would make a local file look incompatible
because no repository hash was known — a bug found and fixed in review.

## Download: verify before ready

````text
precheck (path containment) → stage .part (resume via Range)
   → hash → atomic rename → register as ready
```

- a file with no published SHA-256 is refused (`Unverifiable`);
- a server that ignores `Range` (answers 200) triggers a restart, not a corrupt
  append;
- only a verified file is written to the registry as `ready`.

## Registry

`crates/database/src/registry.rs` stores models, revisions and files in one
transaction. A file row records its state (`pending` / `downloading` /
`verifying` / `ready` / `failed`); an unrecognised state read back from a newer
build is treated as `failed`, never as `ready` — the safe direction.

## Verified against the real thing

````text
$ gguf models pull ggml-org/models --file tinyllamas/stories260K.gguf
repository : ggml-org/models
revision   : 499bc8821c6b12b4e53c5bffcb21ec206f212d81
file       : tinyllamas/stories260K.gguf (1.1 MiB)
verifying  : sha256 270cba1bd5109f42
verified   : sha256 270cba1bd5109f42
registered in the model registry

$ gguf inspect .../stories260K.gguf
  format        : GGUF v3
  architecture  : llama
  quantisation  : F32
  compatibility : compatible_unverified
```

The revision is resolved to a commit before download, the hash matches, and the
registry records the real architecture parsed from the bytes.

## What this task does not answer

| Question | Where it is answered |
| --- | --- |
| Can this model actually run? | Task 6 (llama.cpp adapter loads it) |
| Will it fit in VRAM/RAM? | Task 7 (planner) |
| Token storage / private repos | future (spec §7.3 lists it) |
