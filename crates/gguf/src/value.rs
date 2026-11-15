//! Metadata values and GGML tensor types.

use serde::{Deserialize, Serialize};

/// A value from the GGUF metadata dictionary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Value {
    /// Unsigned 8-bit.
    U8(u8),
    /// Signed 8-bit.
    I8(i8),
    /// Unsigned 16-bit.
    U16(u16),
    /// Signed 16-bit.
    I16(i16),
    /// Unsigned 32-bit.
    U32(u32),
    /// Signed 32-bit.
    I32(i32),
    /// Unsigned 64-bit.
    U64(u64),
    /// Signed 64-bit.
    I64(i64),
    /// 32-bit float.
    F32(f32),
    /// 64-bit float.
    F64(f64),
    /// Boolean.
    Bool(bool),
    /// UTF-8 string.
    String(String),
    /// Homogeneous array.
    Array(Vec<Value>),
}

impl Value {
    /// Numeric view, for fields that are conceptually a count or a length.
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::U8(v) => Some(u64::from(*v)),
            Self::U16(v) => Some(u64::from(*v)),
            Self::U32(v) => Some(u64::from(*v)),
            Self::U64(v) => Some(*v),
            Self::I8(v) if *v >= 0 => Some(*v as u64),
            Self::I16(v) if *v >= 0 => Some(*v as u64),
            Self::I32(v) if *v >= 0 => Some(*v as u64),
            Self::I64(v) if *v >= 0 => Some(*v as u64),
            _ => None,
        }
    }

    /// String view.
    pub fn as_str(&self) -> Option<&'_ str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    /// Array view.
    pub fn as_array(&self) -> Option<&'_ [Value]> {
        match self {
            Self::Array(values) => Some(values),
            _ => None,
        }
    }

    /// Stable type name, for diagnostics.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::U8(_) => "u8",
            Self::I8(_) => "i8",
            Self::U16(_) => "u16",
            Self::I16(_) => "i16",
            Self::U32(_) => "u32",
            Self::I32(_) => "i32",
            Self::U64(_) => "u64",
            Self::I64(_) => "i64",
            Self::F32(_) => "f32",
            Self::F64(_) => "f64",
            Self::Bool(_) => "bool",
            Self::String(_) => "string",
            Self::Array(_) => "array",
        }
    }
}

/// GGML tensor element type.
///
/// The numeric values are part of the GGUF format, so they are spelled out here
/// rather than guessed from a name. `Unknown` keeps the raw id: an unrecognised
/// quantisation is still useful information ("this file uses type 27"), and
/// dropping it would make a compatibility report lie by omission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TensorType {
    /// A type this build knows by name.
    Known(KnownTensorType),
    /// A type id this build does not know.
    Unknown(u32),
}

impl TensorType {
    /// Interpret a raw type id.
    pub fn from_raw(raw: u32) -> Self {
        match KnownTensorType::from_raw(raw) {
            Some(known) => Self::Known(known),
            None => Self::Unknown(raw),
        }
    }

    /// Raw id.
    pub fn raw(self) -> u32 {
        match self {
            Self::Known(known) => known.raw(),
            Self::Unknown(raw) => raw,
        }
    }

    /// Stable name for display; unknown types show their id.
    pub fn name(self) -> String {
        match self {
            Self::Known(known) => known.as_str().to_owned(),
            Self::Unknown(raw) => format!("unknown({raw})"),
        }
    }
}

/// GGML tensor types this build recognises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnownTensorType {
    /// 32-bit float.
    F32,
    /// 16-bit float.
    F16,
    /// 4-bit, one block per 32 values.
    Q4_0,
    /// 4-bit, 0.5% more accurate than Q4_0.
    Q4_1,
    /// 5-bit, legacy.
    Q5_0,
    /// 5-bit, legacy.
    Q5_1,
    /// 8-bit, one block per 32 values.
    Q8_0,
    /// 8-bit.
    Q8_1,
    /// 2-bit k-quant.
    Q2K,
    /// 3-bit k-quant.
    Q3K,
    /// 4-bit k-quant, small.
    Q4K,
    /// 5-bit k-quant, small.
    Q5K,
    /// 6-bit k-quant.
    Q6K,
    /// 8-bit k-quant; used for the output layer of k-quant models.
    Q8K,
    /// 16-bit brain float.
    BF16,
    /// Integer 8-bit (32 values per block).
    I8,
    /// Integer 16-bit.
    I16,
    /// Integer 32-bit.
    I32,
    /// Integer 64-bit.
    I64,
    /// Float 64-bit.
    F64,
    /// Ternary 1.58-bit quantisation.
    Tq1,
    /// Ternary 2-bit quantisation.
    Tq2,
    /// 4-bit, medium k-quant.
    Iq4Nl,
    /// 1-bit i-quant.
    Iq1S,
    /// 1-bit i-quant, medium.
    Iq1M,
    /// 2-bit i-quant, small.
    Iq2Xxs,
    /// 2-bit i-quant, small+.
    Iq2Xs,
    /// 2-bit i-quant.
    Iq2S,
    /// 2-bit i-quant, medium.
    /// 3-bit i-quant, extra small.
    Iq3Xxs,
    /// 3-bit i-quant, small.
    Iq3S,
    /// 4-bit i-quant.
    Iq4Xs,
}

impl KnownTensorType {
    /// Map a raw GGML type id.
    pub fn from_raw(raw: u32) -> Option<Self> {
        Some(match raw {
            0 => Self::F32,
            1 => Self::F16,
            2 => Self::Q4_0,
            3 => Self::Q4_1,
            6 => Self::Q5_0,
            7 => Self::Q5_1,
            8 => Self::Q8_0,
            9 => Self::Q8_1,
            10 => Self::Q2K,
            11 => Self::Q3K,
            12 => Self::Q4K,
            13 => Self::Q5K,
            14 => Self::Q6K,
            15 => Self::Q8K,
            16 => Self::Iq2Xxs,
            17 => Self::Iq2Xs,
            18 => Self::Iq3Xxs,
            19 => Self::Iq1S,
            20 => Self::Iq4Nl,
            21 => Self::Iq3S,
            22 => Self::Iq2S,
            23 => Self::Iq4Xs,
            24 => Self::I8,
            25 => Self::I16,
            26 => Self::I32,
            27 => Self::I64,
            28 => Self::F64,
            29 => Self::Iq1M,
            30 => Self::BF16,
            32 => Self::Tq1,
            33 => Self::Tq2,
            _ => return None,
        })
    }

    /// Raw GGML type id.
    pub fn raw(self) -> u32 {
        match self {
            Self::F32 => 0,
            Self::F16 => 1,
            Self::Q4_0 => 2,
            Self::Q4_1 => 3,
            Self::Q5_0 => 6,
            Self::Q5_1 => 7,
            Self::Q8_0 => 8,
            Self::Q8_1 => 9,
            Self::Q2K => 10,
            Self::Q3K => 11,
            Self::Q4K => 12,
            Self::Q5K => 13,
            Self::Q6K => 14,
            Self::Q8K => 15,
            Self::Iq2Xxs => 16,
            Self::Iq2Xs => 17,
            Self::Iq3Xxs => 18,
            Self::Iq1S => 19,
            Self::Iq4Nl => 20,
            Self::Iq3S => 21,
            Self::Iq2S => 22,
            Self::Iq4Xs => 23,
            Self::I8 => 24,
            Self::I16 => 25,
            Self::I32 => 26,
            Self::I64 => 27,
            Self::F64 => 28,
            Self::Iq1M => 29,
            Self::BF16 => 30,
            Self::Tq1 => 32,
            Self::Tq2 => 33,
        }
    }

    /// Display name, matching the names used in model file names.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::F32 => "F32",
            Self::F16 => "F16",
            Self::Q4_0 => "Q4_0",
            Self::Q4_1 => "Q4_1",
            Self::Q5_0 => "Q5_0",
            Self::Q5_1 => "Q5_1",
            Self::Q8_0 => "Q8_0",
            Self::Q8_1 => "Q8_1",
            Self::Q2K => "Q2_K",
            Self::Q3K => "Q3_K",
            Self::Q4K => "Q4_K",
            Self::Q5K => "Q5_K",
            Self::Q6K => "Q6_K",
            Self::Q8K => "Q8_K",
            Self::BF16 => "BF16",
            Self::I8 => "I8",
            Self::I16 => "I16",
            Self::I32 => "I32",
            Self::I64 => "I64",
            Self::F64 => "F64",
            Self::Tq1 => "TQ1_0",
            Self::Tq2 => "TQ2_0",
            Self::Iq4Nl => "IQ4_NL",
            Self::Iq1S => "IQ1_S",
            Self::Iq1M => "IQ1_M",
            Self::Iq2Xxs => "IQ2_XXS",
            Self::Iq2Xs => "IQ2_XS",
            Self::Iq2S => "IQ2_S",
            Self::Iq3Xxs => "IQ3_XXS",
            Self::Iq3S => "IQ3_S",
            Self::Iq4Xs => "IQ4_XS",
        }
    }

    /// Bits per weight, for size estimation. `None` when it varies by block.
    pub fn bits_per_weight(self) -> Option<f32> {
        Some(match self {
            Self::F32 => 32.0,
            Self::F16 | Self::BF16 => 16.0,
            Self::F64 => 64.0,
            Self::I8 => 8.0,
            Self::I16 => 16.0,
            Self::I32 => 32.0,
            Self::I64 => 64.0,
            Self::Q4_0 | Self::Q4_1 | Self::Iq4Nl => 4.5,
            Self::Q5_0 | Self::Q5_1 => 5.5,
            Self::Q8_0 | Self::Q8_1 => 8.5,
            Self::Q2K => 2.6,
            Self::Q3K => 3.4,
            Self::Q4K => 4.5,
            Self::Q5K => 5.5,
            Self::Q6K => 6.6,
            Self::Q8K => 8.5,
            Self::Tq1 => 1.7,
            Self::Tq2 => 2.1,
            Self::Iq1S => 1.6,
            Self::Iq1M => 1.8,
            Self::Iq2Xxs => 2.1,
            Self::Iq2Xs | Self::Iq2S => 2.4,
            Self::Iq3Xxs => 3.1,
            Self::Iq3S => 3.4,
            Self::Iq4Xs => 4.3,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_types_round_trip_through_their_ids() {
        for raw in [
            0u32, 1, 2, 3, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 20, 30, 32,
        ] {
            let ty = KnownTensorType::from_raw(raw).expect("known type");
            assert_eq!(ty.raw(), raw);
        }
    }

    #[test]
    fn an_unknown_type_keeps_its_id_instead_of_becoming_nothing() {
        let ty = TensorType::from_raw(99);
        assert_eq!(ty, TensorType::Unknown(99));
        assert_eq!(ty.raw(), 99);
        assert_eq!(ty.name(), "unknown(99)");
    }

    #[test]
    fn values_expose_numeric_and_string_views() {
        assert_eq!(Value::U32(7).as_u64(), Some(7));
        assert_eq!(Value::I32(-1).as_u64(), None, "negative is not a count");
        assert_eq!(Value::String("x".into()).as_str(), Some("x"));
        assert_eq!(Value::Bool(true).as_str(), None);
        assert_eq!(Value::U8(1).type_name(), "u8");
    }

    #[test]
    fn quantisation_names_match_the_file_naming_convention() {
        assert_eq!(KnownTensorType::Q4K.as_str(), "Q4_K");
        assert_eq!(KnownTensorType::Iq2Xxs.as_str(), "IQ2_XXS");
        assert_eq!(KnownTensorType::BF16.as_str(), "BF16");
    }
}
