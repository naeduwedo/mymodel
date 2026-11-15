# runtimes/

Runtime manifests and installers.

- `manifests/` — one JSON manifest per runtime version declaring OS, CPU
  architecture, GPU backends, supported model architectures, quantisations,
  parallelism modes, offload and paging capabilities (spec §2.3, §8.1).
- `installers/` — scripts/recipes that fetch and verify a runtime binary
  (source URL, version and SHA-256). Runtimes are installed under the user data
  directory, never inside the application bundle (spec §15).

No runtime is shipped or installed yet; the llama.cpp adapter lands in Task 6.
Nothing here executes code from a model repository.
