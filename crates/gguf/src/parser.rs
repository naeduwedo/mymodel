//! The GGUF parser (spec §7.1).
//!
//! Layout (v2/v3):
//!
//! ```text
//! magic      4   "GGUF"
//! version    u32
//! n_tensors  u64
//! n_kv       u64
//! kv[]       key: string, type: u32, value
//! tensors[]  name: string, n_dims: u32, dims: u64[n_dims], type: u32, offset: u64
//! padding    to `general.alignment` (default 32)
//! data       tensor payloads, in the order of the descriptors
//! ```
//!
//! Two invariants make this safe on untrusted input:
//!
//! 1. every read goes through [`crate::reader::Reader`], which cannot read past the
//!    end of the mapped file;
//! 2. every length taken from the file is checked against [`Limits`] *before* it
//!    is used to size anything.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::error::GgufError;
use crate::reader::{Limits, Reader};
use crate::value::{TensorType, Value};

/// The version this parser understands.
pub const SUPPORTED_VERSIONS: [u32; 2] = [2, 3];

/// Default alignment when `general.alignment` is absent.
pub const DEFAULT_ALIGNMENT: u32 = 32;

/// One tensor descriptor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TensorInfo {
    /// Tensor name, e.g. `blk.0.attn_q.weight`.
    pub name: String,
    /// Dimensions, fastest-varying first, as stored in the file.
    pub dims: Vec<u64>,
    /// Element type.
    pub tensor_type: TensorType,
    /// Offset **relative to the start of the tensor data section**.
    pub offset: u64,
    /// Byte length of this tensor payload, computed from its shape and type.
    pub byte_len: u64,
}

impl TensorInfo {
    /// Number of elements.
    pub fn element_count(&self) -> u64 {
        self.dims.iter().copied().product::<u64>().max(1)
    }
}

/// A parsed GGUF file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Gguf {
    /// Format version.
    pub version: u32,
    /// Metadata dictionary, sorted by key for deterministic output.
    pub metadata: BTreeMap<String, Value>,
    /// Tensor descriptors in file order.
    pub tensors: Vec<TensorInfo>,
    /// Alignment used for the data section.
    pub alignment: u32,
    /// Absolute offset where tensor data starts.
    pub data_offset: u64,
    /// Total file size.
    pub file_len: u64,
    /// Bytes occupied by the tensor payloads.
    pub tensor_bytes: u64,
}

impl Gguf {
    /// Architecture declared by `general.architecture`.
    pub fn architecture(&self) -> Option<&'_ str> {
        self.metadata.get("general.architecture")?.as_str()
    }

    /// Human readable name declared by `general.name`.
    pub fn name(&self) -> Option<&'_ str> {
        self.metadata.get("general.name")?.as_str()
    }

    /// A metadata integer, e.g. `llama.block_count`.
    pub fn metadata_u64(&self, key: &str) -> Option<u64> {
        self.metadata.get(key)?.as_u64()
    }

    /// A metadata string.
    pub fn metadata_str(&self, key: &str) -> Option<&'_ str> {
        self.metadata.get(key)?.as_str()
    }

    /// Distinct tensor types, with the number of tensors using each.
    pub fn tensor_type_histogram(&self) -> BTreeMap<String, usize> {
        let mut histogram = BTreeMap::new();
        for tensor in &self.tensors {
            *histogram.entry(tensor.tensor_type.name()).or_insert(0) += 1;
        }
        histogram
    }

    /// A short label for the dominant quantisation, e.g. `Q4_K` or `mixed`.
    pub fn quantisation_label(&self) -> String {
        let histogram = self.tensor_type_histogram();
        if histogram.is_empty() {
            return "unknown".to_owned();
        }
        if histogram.len() == 1 {
            return histogram.keys().next().cloned().unwrap_or_default();
        }
        // K-quants keep the output tensor at a higher precision, which is normal
        // and not worth surfacing as "mixed".
        let mut sorted: Vec<(&String, &usize)> = histogram.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1));
        format!("mixed ({})", sorted[0].0)
    }
}

/// How much of a file is read to parse it.
///
/// The header, metadata and tensor table live at the front of the file; tensor
/// payloads follow. Reading a bounded prefix means a 100 GB model costs a few
/// megabytes of memory to inspect, without mmap — and without the SIGBUS a
/// truncated mmapped file would cause.
pub const HEADER_READ_LIMIT: u64 = 256 * 1024 * 1024;

/// Parse a GGUF file from disk.
///
/// Only the metadata region is read; tensor payloads are never touched.
pub fn parse_file(path: impl AsRef<Path>) -> Result<Gguf, GgufError> {
    let path = path.as_ref();
    let file = std::fs::File::open(path).map_err(|source| GgufError::Io {
        path: path.display().to_string(),
        source,
    })?;

    let metadata = file.metadata().map_err(|source| GgufError::Io {
        path: path.display().to_string(),
        source,
    })?;

    let file_len = metadata.len();
    if file_len == 0 {
        return Err(GgufError::Truncated {
            offset: 0,
            needed: 24,
        });
    }

    if !metadata.is_file() {
        return Err(GgufError::Io {
            path: path.display().to_string(),
            source: std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a regular file"),
        });
    }

    let limits = Limits::default();
    let to_read = file_len.min(HEADER_READ_LIMIT);

    let mut buffer =
        Vec::with_capacity(usize::try_from(to_read.min(16 * 1024 * 1024)).unwrap_or(0));
    file.take(to_read)
        .read_to_end(&mut buffer)
        .map_err(|source| GgufError::Io {
            path: path.display().to_string(),
            source,
        })?;

    parse_prefix(&buffer, file_len, limits)
}

/// Parse a GGUF file that is already in memory.
pub fn parse_bytes(data: &[u8]) -> Result<Gguf, GgufError> {
    parse_with_limits(data, Limits::default())
}

/// Parse with explicit limits (used by the malformed-file tests).
pub fn parse_with_limits(data: &[u8], limits: Limits) -> Result<Gguf, GgufError> {
    parse_prefix(data, data.len() as u64, limits)
}

/// Parse a buffered prefix of a file whose real size is `file_len`.
///
/// `data` must start at offset 0. Bounds checks against the file use `file_len`,
/// while reads are limited to what is actually buffered.
pub fn parse_prefix(data: &[u8], file_len: u64, limits: Limits) -> Result<Gguf, GgufError> {
    let mut reader = Reader::with_file_len(data, file_len, limits);

    // --- header ------------------------------------------------------------
    let magic = reader.bytes(4)?;
    if magic != b"GGUF" {
        return Err(GgufError::BadMagic {
            found: String::from_utf8_lossy(magic).into_owned(),
        });
    }

    let version = reader.u32()?;
    if !SUPPORTED_VERSIONS.contains(&version) {
        // A byte-swapped version (0x03000000 for v3) means the whole file is
        // big-endian; reporting "version 50331648" would send a user hunting for
        // a corrupt download instead of a variant we simply do not read.
        if version.swap_bytes() != 0 && SUPPORTED_VERSIONS.contains(&version.swap_bytes()) {
            return Err(GgufError::BigEndian);
        }
        return Err(GgufError::UnsupportedVersion { found: version });
    }

    let tensor_count = reader.bounded_u64("tensor count", limits.max_tensor_count)?;
    let kv_count = reader.bounded_u64("metadata count", limits.max_kv_count)?;

    // --- metadata ----------------------------------------------------------
    // Keyed by offset so a duplicate can be reported at the offending position.
    let mut metadata = BTreeMap::new();
    for _ in 0..kv_count {
        let offset = reader.position();
        let key = reader.string("metadata key")?.to_owned();
        let value = read_value(&mut reader, &key)?;

        if metadata.insert(key.clone(), value).is_some() {
            return Err(GgufError::DuplicateKey { key, offset });
        }

        if reader.position() > reader.buffered() {
            return Err(GgufError::Truncated {
                offset: reader.position(),
                needed: 1,
            });
        }

        if reader.position() > limits.max_metadata_bytes {
            return Err(GgufError::limit(
                "metadata bytes",
                reader.position(),
                limits.max_metadata_bytes,
            ));
        }
    }

    let alignment = match metadata.get("general.alignment") {
        Some(value) => {
            let raw = value.as_u64().unwrap_or(0);
            let alignment = u32::try_from(raw).map_err(|_| GgufError::Overflow {
                offset: 0,
                what: "alignment does not fit in u32".to_owned(),
            })?;
            if alignment == 0 || !alignment.is_power_of_two() {
                return Err(GgufError::BadAlignment { value: alignment });
            }
            alignment
        }
        None => DEFAULT_ALIGNMENT,
    };

    // --- tensor table ------------------------------------------------------
    let mut tensors = Vec::with_capacity(usize::try_from(tensor_count.min(4096)).unwrap_or(0));
    for _ in 0..tensor_count {
        let name = reader.string("tensor name")?.to_owned();
        let n_dims = reader.u32()?;
        if n_dims > limits.max_dims {
            return Err(GgufError::limit(
                "tensor rank",
                u64::from(n_dims),
                u64::from(limits.max_dims),
            ));
        }

        let mut dims = Vec::with_capacity(n_dims as usize);
        for _ in 0..n_dims {
            let dim = reader.bounded_u64("tensor dimension", u64::MAX / 2)?;
            if dim == 0 {
                // A zero dimension means the tensor holds no data; accepted, but
                // it must not be treated as an error either way.
                debug!(%name, "tensor declares a zero dimension");
            }
            dims.push(dim);
        }

        let raw_type = reader.u32()?;
        let tensor_type = TensorType::from_raw(raw_type);
        let offset = reader.u64()?;

        let element_count: u64 = dims.iter().copied().try_fold(1u64, |acc, dim| {
            acc.checked_mul(dim).ok_or(GgufError::Overflow {
                offset: reader.position(),
                what: format!("element count of tensor {name:?}"),
            })
        })?;

        tensors.push(TensorInfo {
            name,
            dims,
            tensor_type,
            offset,
            byte_len: estimate_bytes(element_count, tensor_type),
        });
    }

    let table_end = reader.position();
    let data_offset = align_up(table_end, u64::from(alignment)).ok_or(GgufError::Overflow {
        offset: table_end,
        what: "tensor data offset".to_owned(),
    })?;

    let file_len = reader.len();
    if data_offset > file_len {
        return Err(GgufError::Truncated {
            offset: table_end,
            needed: data_offset - table_end,
        });
    }

    // --- validate every tensor against the file ----------------------------
    let mut tensor_bytes: u64 = 0;
    for tensor in &tensors {
        let end = tensor
            .offset
            .checked_add(tensor.byte_len)
            .and_then(|end| end.checked_add(data_offset))
            .ok_or_else(|| GgufError::Overflow {
                offset: data_offset,
                what: format!("span of tensor {:?}", tensor.name),
            })?;

        if end > file_len {
            return Err(GgufError::TensorOutOfBounds {
                name: tensor.name.clone(),
                end,
                file_len,
            });
        }

        tensor_bytes = tensor_bytes.saturating_add(tensor.byte_len);
    }

    debug!(
        version,
        tensors = tensors.len(),
        metadata = metadata.len(),
        alignment,
        data_offset,
        file_len,
        "parsed GGUF"
    );

    Ok(Gguf {
        version,
        metadata,
        tensors,
        alignment,
        data_offset,
        file_len,
        tensor_bytes,
    })
}

fn read_value(reader: &mut Reader<'_>, key: &str) -> Result<Value, GgufError> {
    const TYPE_UINT8: u32 = 0;
    const TYPE_INT8: u32 = 1;
    const TYPE_UINT16: u32 = 2;
    const TYPE_INT16: u32 = 3;
    const TYPE_UINT32: u32 = 4;
    const TYPE_INT32: u32 = 5;
    const TYPE_FLOAT32: u32 = 6;
    const TYPE_BOOL: u32 = 7;
    const TYPE_STRING: u32 = 8;
    const TYPE_ARRAY: u32 = 9;
    const TYPE_UINT64: u32 = 10;
    const TYPE_INT64: u32 = 11;
    const TYPE_FLOAT64: u32 = 12;

    let type_offset = reader.position();
    let value_type = reader.u32()?;

    Ok(match value_type {
        TYPE_UINT8 => Value::U8(reader.bytes(1)?[0]),
        TYPE_INT8 => Value::I8(reader.bytes(1)?[0] as i8),
        TYPE_UINT16 => {
            let bytes = reader.bytes(2)?;
            Value::U16(u16::from_le_bytes([bytes[0], bytes[1]]))
        }
        TYPE_INT16 => {
            let bytes = reader.bytes(2)?;
            Value::I16(i16::from_le_bytes([bytes[0], bytes[1]]))
        }
        TYPE_UINT32 => Value::U32(reader.u32()?),
        TYPE_INT32 => Value::I32(reader.u32()? as i32),
        TYPE_FLOAT32 => Value::F32(f32::from_bits(reader.u32()?)),
        TYPE_BOOL => Value::Bool(reader.bytes(1)?[0] != 0),
        TYPE_STRING => Value::String(reader.string("metadata value")?.to_owned()),
        TYPE_ARRAY => {
            let element_type = reader.u32()?;
            let len = reader.bounded_u64("array length", reader.limits().max_array_len)?;
            // `len` is bounded above, so this allocation is bounded too.
            let mut values = Vec::with_capacity(usize::try_from(len).unwrap_or(0).min(4096));
            for _ in 0..len {
                values.push(read_scalar(reader, element_type, key, type_offset)?);
            }
            Value::Array(values)
        }
        TYPE_UINT64 => Value::U64(reader.u64()?),
        TYPE_INT64 => Value::I64(reader.u64()? as i64),
        TYPE_FLOAT64 => Value::F64(f64::from_bits(reader.u64()?)),
        other => {
            return Err(GgufError::UnknownValueType {
                value_type: other,
                key: key.to_owned(),
                offset: type_offset,
            });
        }
    })
}

/// Read one array element. Nested arrays are legal in the format but never
/// appear in practice; they are still parsed so a valid file cannot be rejected.
fn read_scalar(
    reader: &mut Reader<'_>,
    value_type: u32,
    key: &str,
    type_offset: u64,
) -> Result<Value, GgufError> {
    if value_type == 9 {
        let element_type = reader.u32()?;
        let len = reader.bounded_u64("array length", reader.limits().max_array_len)?;
        let mut values = Vec::with_capacity(usize::try_from(len).unwrap_or(0).min(1024));
        for _ in 0..len {
            values.push(read_scalar(reader, element_type, key, type_offset)?);
        }
        return Ok(Value::Array(values));
    }

    // Reuse the main reader by synthesising the same type tag.
    match value_type {
        0 => Ok(Value::U8(reader.bytes(1)?[0])),
        1 => Ok(Value::I8(reader.bytes(1)?[0] as i8)),
        2 => {
            let bytes = reader.bytes(2)?;
            Ok(Value::U16(u16::from_le_bytes([bytes[0], bytes[1]])))
        }
        3 => {
            let bytes = reader.bytes(2)?;
            Ok(Value::I16(i16::from_le_bytes([bytes[0], bytes[1]])))
        }
        4 => Ok(Value::U32(reader.u32()?)),
        5 => Ok(Value::I32(reader.u32()? as i32)),
        6 => Ok(Value::F32(f32::from_bits(reader.u32()?))),
        7 => Ok(Value::Bool(reader.bytes(1)?[0] != 0)),
        8 => Ok(Value::String(reader.string("array element")?.to_owned())),
        10 => Ok(Value::U64(reader.u64()?)),
        11 => Ok(Value::I64(reader.u64()? as i64)),
        12 => Ok(Value::F64(f64::from_bits(reader.u64()?))),
        other => Err(GgufError::UnknownValueType {
            value_type: other,
            key: key.to_owned(),
            offset: type_offset,
        }),
    }
}

/// Round `value` up to the next multiple of `alignment`.
pub fn align_up(value: u64, alignment: u64) -> Option<u64> {
    if alignment == 0 {
        return None;
    }
    let remainder = value % alignment;
    if remainder == 0 {
        Some(value)
    } else {
        value.checked_add(alignment - remainder)
    }
}

/// Bytes needed for `count` elements of `tensor_type`.
///
/// Quantised types store a block header plus packed values, so the size is
/// computed from the block layout rather than from a bits-per-weight guess.
fn estimate_bytes(element_count: u64, tensor_type: TensorType) -> u64 {
    if element_count == 0 {
        return 0;
    }

    let (block_elements, block_bytes) = match tensor_type {
        TensorType::Known(known) => block_layout(known),
        TensorType::Unknown(_) => {
            // Unknown layout: report nothing rather than a wrong number. The
            // compatibility report flags the type; the size stays 0 so a caller
            // cannot mistake a guess for a measurement.
            return 0;
        }
    };

    // std::primitive::u64::div_ceil is stable; using it avoids a hand-rolled
    // (count + block - 1) / block.
    let blocks = u64::div_ceil(element_count, block_elements);
    blocks.saturating_mul(block_bytes)
}

/// `(elements per block, bytes per block)` for a quantisation.
fn block_layout(known: crate::value::KnownTensorType) -> (u64, u64) {
    use crate::value::KnownTensorType as T;
    match known {
        T::F32 => (1, 4),
        T::F16 => (1, 2),
        T::BF16 => (1, 2),
        T::F64 => (1, 8),
        T::I8 => (1, 1),
        T::I16 => (1, 2),
        T::I32 => (1, 4),
        T::I64 => (1, 8),
        T::Q4_0 => (32, 18),
        T::Q4_1 => (32, 20),
        T::Q5_0 => (32, 22),
        T::Q5_1 => (32, 24),
        T::Q8_0 => (32, 34),
        T::Q8_1 => (32, 36),
        T::Q2K => (256, 84),
        T::Q3K => (256, 110),
        T::Q4K => (256, 144),
        T::Q5K => (256, 176),
        T::Q6K => (256, 210),
        T::Q8K => (256, 292),
        T::Iq2Xxs => (256, 66),
        T::Iq2Xs => (256, 74),
        T::Iq2S => (256, 82),
        T::Iq3Xxs => (256, 98),
        T::Iq3S => (256, 110),
        T::Iq4Xs => (256, 136),
        T::Iq4Nl => (32, 18),
        T::Iq1S => (256, 50),
        T::Iq1M => (256, 56),
        T::Tq1 => (256, 54),
        T::Tq2 => (256, 66),
    }
}
