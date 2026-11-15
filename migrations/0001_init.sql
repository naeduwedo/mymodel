-- 0001_init.sql — schema for GGUF AI Engine (spec §10).
--
-- Conventions that every later migration must keep:
--   * capacities are INTEGER byte counts (never REAL gigabytes);
--   * timestamps are INTEGER UTC epoch milliseconds;
--   * ids are TEXT (UUID v7 for time-ordering, human ids where a user sees them);
--   * STRICT tables, so a wrong type is an error instead of a silent coercion;
--   * foreign keys are declared here and enforced per connection
--     (PRAGMA foreign_keys = ON — see Database::open).

PRAGMA foreign_keys = ON;

-- ---------------------------------------------------------------- models ---

CREATE TABLE models (
    model_id        TEXT    NOT NULL PRIMARY KEY,
    publisher       TEXT    NOT NULL,
    display_name    TEXT    NOT NULL,
    architecture    TEXT,
    modalities_json TEXT    NOT NULL DEFAULT '[]',
    license         TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    deleted_at      INTEGER
) STRICT;

CREATE INDEX idx_models_publisher ON models (publisher);

CREATE TABLE model_revisions (
    revision_id   TEXT    NOT NULL PRIMARY KEY,
    model_id      TEXT    NOT NULL REFERENCES models (model_id) ON DELETE CASCADE,
    revision      TEXT    NOT NULL,
    resolved_at   INTEGER NOT NULL,
    manifest_path TEXT,
    UNIQUE (model_id, revision)
) STRICT;

CREATE TABLE model_files (
    file_id       TEXT    NOT NULL PRIMARY KEY,
    revision_id   TEXT    NOT NULL REFERENCES model_revisions (revision_id) ON DELETE CASCADE,
    path          TEXT    NOT NULL,
    role          TEXT    NOT NULL,
    required      INTEGER NOT NULL DEFAULT 1 CHECK (required IN (0, 1)),
    size_bytes    INTEGER NOT NULL CHECK (size_bytes >= 0),
    sha256        TEXT,
    -- pending | downloading | verifying | ready | failed
    state         TEXT    NOT NULL DEFAULT 'pending',
    downloaded_at INTEGER,
    UNIQUE (revision_id, path)
) STRICT;

CREATE INDEX idx_model_files_sha256 ON model_files (sha256);

CREATE TABLE model_components (
    component_id TEXT NOT NULL PRIMARY KEY,
    revision_id  TEXT NOT NULL REFERENCES model_revisions (revision_id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    role         TEXT NOT NULL,
    required     INTEGER NOT NULL DEFAULT 1 CHECK (required IN (0, 1)),
    present      INTEGER NOT NULL DEFAULT 0 CHECK (present IN (0, 1)),
    file_id      TEXT REFERENCES model_files (file_id) ON DELETE SET NULL
) STRICT;

CREATE TABLE model_manifests (
    manifest_id    TEXT    NOT NULL PRIMARY KEY,
    revision_id    TEXT    NOT NULL REFERENCES model_revisions (revision_id) ON DELETE CASCADE,
    schema_version INTEGER NOT NULL,
    body_json      TEXT    NOT NULL,
    created_at     INTEGER NOT NULL,
    UNIQUE (revision_id, schema_version)
) STRICT;

-- -------------------------------------------------------------- runtimes ---

CREATE TABLE runtimes (
    runtime_id     TEXT    NOT NULL PRIMARY KEY,
    version        TEXT    NOT NULL,
    install_path   TEXT,
    sha256         TEXT,
    installed      INTEGER NOT NULL DEFAULT 0 CHECK (installed IN (0, 1)),
    installed_at   INTEGER,
    UNIQUE (runtime_id, version)
) STRICT;

CREATE TABLE runtime_capabilities (
    runtime_id  TEXT NOT NULL REFERENCES runtimes (runtime_id) ON DELETE CASCADE,
    capability  TEXT NOT NULL,
    -- supported | unsupported | experimental | unknown | planned
    status      TEXT NOT NULL,
    detail      TEXT,
    probed_at   INTEGER NOT NULL,
    PRIMARY KEY (runtime_id, capability)
) STRICT;

-- --------------------------------------------------------------- devices ---

CREATE TABLE devices (
    device_id            TEXT    NOT NULL PRIMARY KEY,
    name                 TEXT    NOT NULL,
    vendor               TEXT    NOT NULL,
    backend              TEXT    NOT NULL,
    total_vram_bytes     INTEGER CHECK (total_vram_bytes IS NULL OR total_vram_bytes >= 0),
    free_vram_bytes      INTEGER CHECK (free_vram_bytes IS NULL OR free_vram_bytes >= 0),
    memory_bandwidth_bytes_sec INTEGER,
    compute_capability   TEXT,
    pci_bus_id           TEXT,
    driver_version       TEXT,
    numa_node            INTEGER,
    -- detection provenance: which probe produced this row
    detected_by          TEXT    NOT NULL,
    probed_at            INTEGER NOT NULL
) STRICT;

CREATE TABLE gpu_topology (
    from_device TEXT    NOT NULL REFERENCES devices (device_id) ON DELETE CASCADE,
    to_device   TEXT    NOT NULL REFERENCES devices (device_id) ON DELETE CASCADE,
    interconnect TEXT   NOT NULL,
    p2p_detected INTEGER NOT NULL CHECK (p2p_detected IN (0, 1)),
    p2p_driver_supported INTEGER NOT NULL CHECK (p2p_driver_supported IN (0, 1)),
    p2p_tested  INTEGER NOT NULL CHECK (p2p_tested IN (0, 1)),
    measured_bandwidth_bytes_sec INTEGER,
    probed_at   INTEGER NOT NULL,
    PRIMARY KEY (from_device, to_device)
) STRICT;

CREATE TABLE gpu_allocations (
    allocation_id   TEXT NOT NULL PRIMARY KEY,
    lease_id        TEXT NOT NULL,
    device_id       TEXT NOT NULL REFERENCES devices (device_id) ON DELETE CASCADE,
    model_instance_id TEXT,
    bytes_reserved  INTEGER NOT NULL CHECK (bytes_reserved >= 0),
    created_at      INTEGER NOT NULL,
    released_at     INTEGER,
    expires_at      INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_gpu_allocations_device ON gpu_allocations (device_id, released_at);

-- -------------------------------------------------------------- instances ---

CREATE TABLE model_instances (
    instance_id   TEXT NOT NULL PRIMARY KEY,
    model_id      TEXT NOT NULL,
    revision_id   TEXT,
    runtime_id    TEXT NOT NULL,
    worker_pid    INTEGER,
    -- execution strategy from spec §5.1
    strategy      TEXT NOT NULL,
    devices_json  TEXT NOT NULL DEFAULT '[]',
    -- starting | loaded | unloading | failed | stopped
    state         TEXT NOT NULL DEFAULT 'starting',
    started_at    INTEGER NOT NULL,
    stopped_at    INTEGER,
    error_message TEXT
) STRICT;

CREATE INDEX idx_model_instances_model ON model_instances (model_id, state);

CREATE TABLE memory_allocations (
    allocation_id  TEXT NOT NULL PRIMARY KEY,
    instance_id    TEXT NOT NULL REFERENCES model_instances (instance_id) ON DELETE CASCADE,
    -- gpu_vram | host_ram | nvme_cache
    tier           TEXT NOT NULL,
    device_id      TEXT,
    weights_bytes  INTEGER NOT NULL DEFAULT 0 CHECK (weights_bytes >= 0),
    kv_cache_bytes INTEGER NOT NULL DEFAULT 0 CHECK (kv_cache_bytes >= 0),
    workspace_bytes INTEGER NOT NULL DEFAULT 0 CHECK (workspace_bytes >= 0),
    created_at     INTEGER NOT NULL,
    released_at    INTEGER
) STRICT;

CREATE TABLE tensor_pages (
    model_id        TEXT    NOT NULL,
    tensor_name     TEXT    NOT NULL,
    file_id         TEXT    NOT NULL,
    file_offset     INTEGER NOT NULL CHECK (file_offset >= 0),
    byte_len        INTEGER NOT NULL CHECK (byte_len >= 0),
    quant_type      TEXT,
    layer_index     INTEGER,
    expert_index    INTEGER,
    -- nvme | host_ram | gpu | runtime_managed | unknown
    placement       TEXT    NOT NULL,
    device_id       TEXT,
    pin_count       INTEGER NOT NULL DEFAULT 0 CHECK (pin_count >= 0),
    last_access_epoch INTEGER NOT NULL DEFAULT 0,
    access_count    INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (model_id, tensor_name, file_id)
) STRICT;

-- ----------------------------------------------------------------- tasks ---

CREATE TABLE inference_tasks (
    task_id          TEXT    NOT NULL PRIMARY KEY,
    model_id         TEXT    NOT NULL,
    runtime_id       TEXT,
    modality         TEXT    NOT NULL,
    instance_id      TEXT,
    request_json     TEXT    NOT NULL,
    plan_json        TEXT,
    -- queued | preparing | downloading | loading | running | postprocessing |
    -- completed | failed | cancelled
    status           TEXT    NOT NULL,
    progress         REAL    NOT NULL DEFAULT 0 CHECK (progress >= 0 AND progress <= 1),
    output_refs_json TEXT    NOT NULL DEFAULT '[]',
    error_code       TEXT,
    error_message    TEXT,
    created_at       INTEGER NOT NULL,
    started_at       INTEGER,
    finished_at      INTEGER
) STRICT;

CREATE INDEX idx_inference_tasks_status ON inference_tasks (status, created_at);

CREATE TABLE inference_sessions (
    session_id     TEXT NOT NULL PRIMARY KEY,
    client         TEXT NOT NULL,
    started_at     INTEGER NOT NULL,
    last_seen_at   INTEGER NOT NULL,
    last_event_seq INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE TABLE download_tasks (
    download_id    TEXT NOT NULL PRIMARY KEY,
    repo_id        TEXT NOT NULL,
    revision       TEXT NOT NULL,
    file_path      TEXT NOT NULL,
    total_bytes    INTEGER NOT NULL CHECK (total_bytes >= 0),
    received_bytes INTEGER NOT NULL DEFAULT 0 CHECK (received_bytes >= 0),
    -- pending | running | paused | verifying | completed | failed | cancelled
    status         TEXT NOT NULL DEFAULT 'pending',
    attempts       INTEGER NOT NULL DEFAULT 0,
    etag           TEXT,
    sha256         TEXT,
    error_message  TEXT,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_download_tasks_status ON download_tasks (status, created_at);

-- ----------------------------------------------------------- telemetry -----

CREATE TABLE performance_metrics (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id     TEXT REFERENCES inference_tasks (task_id) ON DELETE CASCADE,
    instance_id TEXT,
    metric      TEXT    NOT NULL,
    value       REAL    NOT NULL,
    recorded_at INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_performance_metrics_task ON performance_metrics (task_id, metric);

CREATE TABLE system_events (
    seq       INTEGER PRIMARY KEY AUTOINCREMENT,
    at        INTEGER NOT NULL,
    level     TEXT    NOT NULL,
    category  TEXT    NOT NULL,
    message   TEXT    NOT NULL,
    detail_json TEXT
) STRICT;

CREATE INDEX idx_system_events_at ON system_events (at);

CREATE TABLE settings (
    key        TEXT NOT NULL PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;
