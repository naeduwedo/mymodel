# proto/

Protobuf definitions for the core ⇄ UI and core ⇄ worker protocols (Task 2).

Planned files (spec §12): `common.proto`, `hardware.proto`, `gpu.proto`,
`memory.proto`, `model.proto`, `runtime.proto`, `inference.proto`,
`task.proto`.

Until they exist, cross-component types live in `crates/protocol`, which also
carries the protocol version used for the client handshake.
