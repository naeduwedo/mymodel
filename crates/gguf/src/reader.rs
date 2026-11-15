//! Bounds-checked little-endian reader.
//!
//! Every read goes through this type. That is deliberate: a GGUF file is
//! untrusted input (spec §7.1), so the only way to be sure there is no unchecked
//! indexing is to make unchecked indexing impossible by construction — there is
//! no `&[u8]` handed out except through these methods.
//!
//! Limits exist because a 20-byte header can claim a 2^60-element array. A parser
//! that "allocates what the file says" is a remote memory-exhaustion bug.

use crate::error::GgufError;

/// Hard limits applied to anything a file declares.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Maximum number of metadata key/value pairs.
    pub max_kv_count: u64,
    /// Maximum number of tensors.
    pub max_tensor_count: u64,
    /// Maximum length of a single string (key, name, value).
    pub max_string_len: u64,
    /// Maximum number of elements in one metadata array.
    pub max_array_len: u64,
    /// Maximum tensor rank.
    pub max_dims: u32,
    /// Maximum total metadata bytes (KV + tensor table).
    pub max_metadata_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            // Real files: ~100 KV pairs, ~1000 tensors, a few MiB of metadata.
            // These leave three orders of magnitude of headroom while still
            // bounding a malformed header.
            max_kv_count: 1 << 20,
            max_tensor_count: 1 << 20,
            max_string_len: 64 * 1024 * 1024,
            max_array_len: 1 << 24,
            max_dims: 8,
            max_metadata_bytes: 256 * 1024 * 1024,
        }
    }
}

/// Cursor over a byte slice that cannot read outside it.
#[derive(Debug)]
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    limits: Limits,
    /// Size of the file on disk, which may exceed the buffered prefix.
    file_len: u64,
}

impl<'a> Reader<'a> {
    /// Create a reader over the whole file.
    pub fn new(data: &'a [u8], limits: Limits) -> Self {
        Self::with_file_len(data, data.len() as u64, limits)
    }

    /// Create a reader over a buffer holding only a prefix of the file.
    pub fn with_file_len(data: &'a [u8], file_len: u64, limits: Limits) -> Self {
        Self {
            data,
            pos: 0,
            limits,
            file_len,
        }
    }

    /// Current offset from the start of the file.
    pub fn position(&self) -> u64 {
        self.pos as u64
    }

    /// Size of the file on disk (may exceed the buffered prefix).
    pub fn len(&self) -> u64 {
        self.file_len
    }

    /// Bytes actually available in memory.
    pub fn buffered(&self) -> u64 {
        self.data.len() as u64
    }

    /// True when the whole file has been consumed.
    pub fn is_exhausted(&self) -> bool {
        self.pos >= self.data.len()
    }

    /// True when there is nothing buffered.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// The limits in force.
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Remaining bytes.
    pub fn remaining(&self) -> u64 {
        (self.data.len() - self.pos) as u64
    }

    /// Read exactly `len` bytes.
    pub fn bytes(&mut self, len: u64) -> Result<&'a [u8], GgufError> {
        let offset = self.pos as u64;

        // `len` comes from the file, so it may not fit in usize on 32-bit hosts.
        let len_usize = usize::try_from(len).map_err(|_| GgufError::Overflow {
            offset,
            what: format!("requested length {len} does not fit in memory"),
        })?;

        let end = self.pos.checked_add(len_usize).ok_or(GgufError::Overflow {
            offset,
            what: "byte range end".to_owned(),
        })?;

        if end > self.data.len() {
            return Err(GgufError::Truncated {
                offset,
                needed: len,
            });
        }

        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    /// Skip `len` bytes.
    pub fn skip(&mut self, len: u64) -> Result<(), GgufError> {
        self.bytes(len).map(|_| ())
    }

    /// Read a `u32`.
    pub fn u32(&mut self) -> Result<u32, GgufError> {
        let bytes = self.bytes(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Read a `u64`.
    pub fn u64(&mut self) -> Result<u64, GgufError> {
        let bytes = self.bytes(8)?;
        let mut array = [0u8; 8];
        array.copy_from_slice(bytes);
        Ok(u64::from_le_bytes(array))
    }

    /// Read a `u64` and confirm it is within a limit.
    pub fn bounded_u64(&mut self, what: &'static str, limit: u64) -> Result<u64, GgufError> {
        let value = self.u64()?;
        if value > limit {
            return Err(GgufError::limit(what, value, limit));
        }
        Ok(value)
    }

    /// Read a length-prefixed UTF-8 string.
    ///
    /// The length is checked against the limit *before* anything is allocated,
    /// and the string borrows the mapped file rather than copying it.
    pub fn string(&mut self, context: &str) -> Result<&'a str, GgufError> {
        let offset = self.pos as u64;
        let len = self.bounded_u64("string length", self.limits.max_string_len)?;
        let bytes = self.bytes(len)?;
        std::str::from_utf8(bytes).map_err(|_| GgufError::InvalidUtf8 {
            context: context.to_owned(),
            offset,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_primitive_values() {
        let data = [1u8, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0];
        let mut reader = Reader::new(&data, Limits::default());
        assert_eq!(reader.u32().unwrap(), 1);
        assert_eq!(reader.u64().unwrap(), 2);
        assert!(reader.is_exhausted());
    }

    #[test]
    fn refuses_to_read_past_the_end() {
        let data = [1u8, 0, 0];
        let mut reader = Reader::new(&data, Limits::default());
        let err = reader.u32().unwrap_err();
        assert!(matches!(err, GgufError::Truncated { .. }));
    }

    #[test]
    fn a_huge_declared_length_is_rejected_before_allocating() {
        // 2^60-byte string: the parser must refuse on the length, not try to read.
        let mut data = (1u64 << 60).to_le_bytes().to_vec();
        data.extend_from_slice(b"abcd");
        let mut reader = Reader::new(&data, Limits::default());
        let err = reader.string("test").unwrap_err();
        match err {
            GgufError::Limit(limit) => {
                assert_eq!(limit.value, 1 << 60);
                assert!(limit.limit < 1 << 60);
            }
            other => panic!("expected a limit error, got {other:?}"),
        }
    }

    #[test]
    fn arrays_and_strings_are_length_prefixed() {
        let mut data = 3u64.to_le_bytes().to_vec();
        data.extend_from_slice(b"abc");
        let mut reader = Reader::new(&data, Limits::default());
        assert_eq!(reader.string("key").unwrap(), "abc");
    }

    #[test]
    fn invalid_utf8_is_reported_with_context() {
        let mut data = 2u64.to_le_bytes().to_vec();
        data.extend_from_slice(&[0xff, 0xfe]);
        let mut reader = Reader::new(&data, Limits::default());
        let err = reader.string("metadata key").unwrap_err();
        match err {
            GgufError::InvalidUtf8 { context, .. } => assert_eq!(context, "metadata key"),
            other => panic!("expected invalid utf8, got {other:?}"),
        }
    }

    #[test]
    fn offsets_that_overflow_are_reported() {
        let data = vec![0u8; 16];
        let mut reader = Reader::new(&data, Limits::default());
        let err = reader.bytes(u64::MAX).unwrap_err();
        assert!(matches!(
            err,
            GgufError::Overflow { .. } | GgufError::Truncated { .. }
        ));
    }
}
