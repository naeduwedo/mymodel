//! Parser tests against a real file and against deliberately malformed input.
//!
//! Two kinds of test here, and the distinction matters:
//!
//! * **Real file.** If a GGUF sample exists on this machine, parse it and assert
//!   what is actually in it. The test fails loudly when the sample is missing,
//!   because a test that silently passes without running is worse than none.
//! * **Synthetic malformed files.** Built byte by byte here so every guard has a
//!   case: bad magic, unsupported version, truncation, absurd lengths, duplicate
//!   keys, out-of-range tensors, bad alignment.

use gguf_engine_gguf::{
    GgufError, KnownTensorType, Limits, TensorType, parse_bytes, parse_file, parse_with_limits,
};

/// Sample locations, most specific first.
const SAMPLES: &[&str] =
    &["F:/dsh-project/rustMoe/fixtures/tiny-ds41/tiny-ds41-00001-of-00001.gguf"];

fn sample_path() -> Option<&'static str> {
    SAMPLES
        .iter()
        .copied()
        .find(|path| std::path::Path::new(path).is_file())
}
// ---------------------------------------------------------------- builders --

/// Minimal valid GGUF: one u32 metadata entry, no tensors.
fn minimal_gguf() -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(b"GGUF");
    data.extend_from_slice(&3u32.to_le_bytes());
    data.extend_from_slice(&0u64.to_le_bytes());
    data.extend_from_slice(&1u64.to_le_bytes());

    let key = b"general.alignment";
    data.extend_from_slice(&(key.len() as u64).to_le_bytes());
    data.extend_from_slice(key);
    data.extend_from_slice(&4u32.to_le_bytes());
    data.extend_from_slice(&32u32.to_le_bytes());
    data
}

fn with_u64_at(mut data: Vec<u8>, offset: usize, value: u64) -> Vec<u8> {
    data[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    data
}

fn one_tensor_gguf(
    name: &[u8],
    dims: &[u64],
    tensor_type: u32,
    offset: u64,
    padding: usize,
) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(b"GGUF");
    data.extend_from_slice(&3u32.to_le_bytes());
    data.extend_from_slice(&1u64.to_le_bytes());
    data.extend_from_slice(&0u64.to_le_bytes());

    data.extend_from_slice(&(name.len() as u64).to_le_bytes());
    data.extend_from_slice(name);
    data.extend_from_slice(&(dims.len() as u32).to_le_bytes());
    for dim in dims {
        data.extend_from_slice(&dim.to_le_bytes());
    }
    data.extend_from_slice(&tensor_type.to_le_bytes());
    data.extend_from_slice(&offset.to_le_bytes());

    // The data section starts at the next alignment boundary, so the buffer must
    // reach it even when the tensor itself is empty.
    let table_end = data.len();
    let data_start = usize::div_ceil(table_end, 32) * 32;
    data.resize(data_start, 0);
    data.extend_from_slice(&vec![0u8; padding]);
    data
}

// -------------------------------------------------------------- real file ---

#[test]
fn parses_a_real_gguf_from_this_machine() {
    let path = sample_path().unwrap_or_else(|| {
        panic!(
            "no GGUF sample found at {:?}; this test must not pass without running",
            SAMPLES
        )
    });

    let gguf = parse_file(path).expect("sample parses");

    assert!(gguf.version >= 2, "version {}", gguf.version);
    assert!(
        !gguf.tensors.is_empty(),
        "the sample should contain tensors"
    );
    assert!(
        !gguf.metadata.is_empty(),
        "the sample should contain metadata"
    );

    let architecture = gguf.architecture().expect("general.architecture");
    assert!(!architecture.is_empty());
    assert!(!architecture.contains(" "), "{architecture}");

    assert!(gguf.alignment.is_power_of_two());
    assert!(gguf.data_offset <= gguf.file_len);

    for tensor in &gguf.tensors {
        assert!(!tensor.name.is_empty());
        assert!(!tensor.dims.is_empty());
        let end = gguf.data_offset + tensor.offset + tensor.byte_len;
        assert!(
            end <= gguf.file_len,
            "tensor {} ends at {end}, file is {}",
            tensor.name,
            gguf.file_len
        );
    }

    let label = gguf.quantisation_label();
    assert!(!label.is_empty());
    assert!(!label.contains("unknown"), "{label}");

    println!(
        "parsed {}: v{}, {} tensors, {} metadata entries, alignment {}, quant {}",
        path,
        gguf.version,
        gguf.tensors.len(),
        gguf.metadata.len(),
        gguf.alignment,
        label
    );
}

#[test]
fn parsing_the_same_file_twice_gives_identical_results() {
    let path = sample_path().expect("GGUF sample");
    let first = parse_file(path).unwrap();
    let second = parse_file(path).unwrap();
    assert_eq!(first, second, "parsing must be deterministic");
}
// ---------------------------------------------------------------- failures --

#[test]
fn rejects_a_file_whose_magic_is_wrong() {
    let mut data = minimal_gguf();
    data[0] = b"X"[0];
    match parse_bytes(&data) {
        Err(GgufError::BadMagic { found }) => assert!(found.starts_with("X")),
        other => panic!("expected BadMagic, got {other:?}"),
    }
}

#[test]
fn rejects_an_unsupported_version() {
    let mut data = minimal_gguf();
    data[4..8].copy_from_slice(&1u32.to_le_bytes());
    match parse_bytes(&data) {
        Err(GgufError::UnsupportedVersion { found }) => assert_eq!(found, 1),
        other => panic!("expected UnsupportedVersion, got {other:?}"),
    }
}

#[test]
fn a_big_endian_file_is_named_as_such() {
    // Real files with a "-be" suffix in their name look like this: the version
    // field is byte-swapped, so a naive reader reports version 50331648.
    let mut data = minimal_gguf();
    data[4..8].copy_from_slice(&3u32.to_be_bytes());
    match parse_bytes(&data) {
        Err(GgufError::BigEndian) => {}
        other => panic!("expected BigEndian, got {other:?}"),
    }
}

#[test]
fn rejects_a_truncated_header() {
    let data = minimal_gguf();
    for cut in [0, 1, 3, 4, 8, 16, 23] {
        let err = parse_bytes(&data[..cut]).unwrap_err();
        assert!(
            matches!(
                err,
                GgufError::Truncated { .. } | GgufError::BadMagic { .. }
            ),
            "cut at {cut} gave {err:?}"
        );
    }
}

#[test]
fn refuses_an_absurd_metadata_count_without_allocating() {
    let data = with_u64_at(minimal_gguf(), 16, 1 << 40);
    let err = parse_bytes(&data).unwrap_err();
    assert!(matches!(err, GgufError::Limit(_)), "got {err:?}");
}

#[test]
fn refuses_an_absurd_tensor_count_without_allocating() {
    let data = with_u64_at(minimal_gguf(), 8, 1 << 40);
    let err = parse_bytes(&data).unwrap_err();
    assert!(matches!(err, GgufError::Limit(_)), "got {err:?}");
}

#[test]
fn refuses_a_string_longer_than_the_file() {
    let data = with_u64_at(minimal_gguf(), 24, 1 << 40);
    let err = parse_bytes(&data).unwrap_err();
    assert!(matches!(err, GgufError::Limit(_)), "got {err:?}");
}

#[test]
fn refuses_duplicate_metadata_keys() {
    let mut data = minimal_gguf();
    data = with_u64_at(data, 16, 2);

    let key = b"general.alignment";
    data.extend_from_slice(&(key.len() as u64).to_le_bytes());
    data.extend_from_slice(key);
    data.extend_from_slice(&4u32.to_le_bytes());
    data.extend_from_slice(&64u32.to_le_bytes());

    match parse_bytes(&data) {
        Err(GgufError::DuplicateKey { key, .. }) => assert_eq!(key, "general.alignment"),
        other => panic!("expected DuplicateKey, got {other:?}"),
    }
}

#[test]
fn refuses_an_alignment_that_is_not_a_power_of_two() {
    let mut data = minimal_gguf();
    let value_offset = 24 + 8 + "general.alignment".len() + 4;
    data[value_offset..value_offset + 4].copy_from_slice(&24u32.to_le_bytes());
    match parse_bytes(&data) {
        Err(GgufError::BadAlignment { value }) => assert_eq!(value, 24),
        other => panic!("expected BadAlignment, got {other:?}"),
    }
}

#[test]
fn refuses_an_unknown_metadata_type() {
    let mut data = minimal_gguf();
    let type_offset = 24 + 8 + "general.alignment".len();
    data[type_offset..type_offset + 4].copy_from_slice(&77u32.to_le_bytes());
    match parse_bytes(&data) {
        Err(GgufError::UnknownValueType { value_type, .. }) => assert_eq!(value_type, 77),
        other => panic!("expected UnknownValueType, got {other:?}"),
    }
}

#[test]
fn tensor_pointing_past_the_end_is_rejected() {
    let data = one_tensor_gguf(b"blk.0.attn_q.weight", &[1 << 20], 1, 0, 0);
    match parse_bytes(&data) {
        Err(GgufError::TensorOutOfBounds { name, .. }) => {
            assert_eq!(name, "blk.0.attn_q.weight");
        }
        other => panic!("expected TensorOutOfBounds, got {other:?}"),
    }
}

#[test]
fn element_count_overflow_is_rejected() {
    let data = one_tensor_gguf(b"huge", &[u64::MAX / 2, u64::MAX / 2], 1, 0, 0);
    let err = parse_bytes(&data).unwrap_err();
    assert!(
        matches!(
            err,
            GgufError::Overflow { .. } | GgufError::TensorOutOfBounds { .. }
        ),
        "got {err:?}"
    );
}

#[test]
fn zero_dimension_tensor_is_accepted_but_contributes_no_bytes() {
    let data = one_tensor_gguf(b"empty", &[0], 1, 0, 0);
    let gguf = parse_bytes(&data).expect("zero-length tensor is legal");
    assert_eq!(gguf.tensors[0].byte_len, 0);
}

#[test]
fn a_tensor_type_this_build_does_not_know_keeps_its_id() {
    let data = one_tensor_gguf(b"weird", &[4], 999, 0, 0);
    let gguf = parse_bytes(&data).expect("unknown tensor type must not fail parsing");
    assert_eq!(gguf.tensors[0].tensor_type, TensorType::Unknown(999));
    assert_eq!(gguf.quantisation_label(), "unknown(999)");
    assert_eq!(
        gguf.tensors[0].byte_len, 0,
        "an unknown layout must not be sized by guessing"
    );
}

#[test]
fn an_array_length_beyond_the_limit_is_refused() {
    let limits = Limits {
        max_array_len: 4,
        ..Limits::default()
    };
    let mut data = minimal_gguf();
    let type_offset = 24 + 8 + "general.alignment".len();
    data[type_offset..type_offset + 4].copy_from_slice(&9u32.to_le_bytes());
    data.extend_from_slice(&8u32.to_le_bytes());
    data.extend_from_slice(&(1u64 << 33).to_le_bytes());

    let err = parse_with_limits(&data, limits).unwrap_err();
    assert!(matches!(err, GgufError::Limit(_)), "got {err:?}");
}

#[test]
fn quantised_sizes_use_the_real_block_layout() {
    let data = one_tensor_gguf(b"q4k", &[256 * 2], KnownTensorType::Q4K.raw(), 0, 1024);
    let gguf = parse_bytes(&data).expect("parses");
    assert_eq!(
        gguf.tensors[0].byte_len,
        2 * 144,
        "two Q4_K blocks are 288 bytes"
    );
}

#[test]
fn a_prefix_shorter_than_the_metadata_is_reported_as_truncated() {
    // Simulate reading only the first 30 bytes of a bigger file.
    let data = minimal_gguf();
    let err = gguf_engine_gguf::parse_prefix(&data[..30], data.len() as u64, Limits::default())
        .unwrap_err();
    assert!(matches!(err, GgufError::Truncated { .. }), "got {err:?}");
}

#[test]
fn a_file_that_is_not_a_file_is_rejected_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let err = parse_file(dir.path()).unwrap_err();
    matches!(err, GgufError::Io { .. });
}
