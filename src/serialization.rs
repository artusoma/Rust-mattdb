//! Module for serializing / deserializing rows

use std::fmt::Write;

#[macro_export]
macro_rules! to_rust_type {
    ($stream:ident, $data_type:expr, $data_value:pat) => {
        let res = Deserializer::deserialize_next(&mut $stream, $data_type);
        let $data_value = (match res {
            Ok(x) => x,
            Err(_) => panic!(),
        }) else {
            panic!()
        };
    };
}
pub use to_rust_type as to_rust_type;

/// Errors while performing data deserialization
#[derive(thiserror::Error, Debug, PartialEq, PartialOrd)]
pub enum DeserializationError {
    #[error("Error deserializing string: {0}")]
    StringDeserializeError(String),
    #[error("Error deserializing int: {0}")]
    IntDeserializeError(String),
    #[error("Unexpected end to buffer while reading")]
    BufferUnexpectedEnd,
}

/// Errors while performing data serialization
#[derive(thiserror::Error, Debug, PartialEq)]
pub enum SerializationError {
    #[error("Error serializing: {0}")]
    SerializtionError(String),
    #[error("String overflow: expected max length {0}, got {1}")]
    StringOverflow(usize, usize),
    #[error("Mismatched types: got type {0} and value {1}")]
    TypeMismatch(DataType, DataValue),
    #[error("Unexpected end to buffer while writing")]
    BufferUnexpectedEnd,
}

/// Stream of bytes to be used in Serializer
#[derive(Debug)]
struct WriteByteStream {
    buffer: Vec<u8>,
    position: usize,
    length: usize,
}

impl WriteByteStream {
    fn new(size: usize) -> Self {
        WriteByteStream {
            buffer: vec![0u8; size],
            position: 0,
            length: size,
        }
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), SerializationError> {
        let start = self.position;
        let end = start + bytes.len();
        self.position += bytes.len();

        if end > self.length {
            Err(SerializationError::BufferUnexpectedEnd)
        } else {
            self.buffer[start..end].copy_from_slice(bytes);
            Ok(())
        }
    }

    fn pad(&mut self, padding: usize) -> Result<(), SerializationError> {
        self.position += padding;
        if self.position > self.length {
            Err(SerializationError::BufferUnexpectedEnd)
        } else {
            Ok(())
        }
    }
}

/// Stream of bytes to be used in Deserializer
#[derive(Debug)]
pub struct ReadByteStream<'a> {
    bytes: &'a [u8],
    position: usize,
    length: usize,
}

impl<'a> ReadByteStream<'a> {
    /// Get next `len` bytes from data and advance the position, checking
    /// if we are out of bounds to prevent a panic!
    pub fn read(&mut self, len: usize) -> Result<&[u8], DeserializationError> {
        let start = self.position;
        let end = start + len;
        self.position += len;

        if end > self.length {
            Err(DeserializationError::BufferUnexpectedEnd)
        } else {
            Ok(&self.bytes[start..end])
        }
    }

    pub fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            position: 0,
            length: bytes.len(),
        }
    }
}

/// Enum to represent Rust values from table dtypes
#[derive(Debug, PartialEq, Clone)]
pub enum DataValue {
    Char(String),
    Int(i64),
}

impl std::fmt::Display for DataValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Char(s) => write!(f, "{}", s),
            Self::Int(x) => write!(f, "{}", x),
        }
    }
}

/// Enum to represent possible dtypes in a table
///
/// Will handle the serialization and deserialization of data
#[derive(Debug, PartialEq, Clone)]
pub enum DataType {
    Char(usize),
    Int,
}

impl DataType {
    /// Deserialize a byte stream to a DataValue
    pub fn deserialize(
        &self,
        stream: &mut ReadByteStream,
    ) -> Result<DataValue, DeserializationError> {
        use DeserializationError::*;
        let bytes = stream.read(self.size())?;
        match self {
            Self::Char(x) => Ok(DataValue::Char(
                str::from_utf8(bytes)
                    .map_err(|e| StringDeserializeError(e.to_string()))?
                    .trim_end_matches('\0')
                    .to_owned(),
            )),
            Self::Int => Ok(DataValue::Int(i64::from_be_bytes(
                bytes
                    .try_into()
                    .map_err(|e: std::array::TryFromSliceError| {
                        IntDeserializeError(e.to_string())
                    })?,
            ))),
        }
    }

    /// Serializes a value to the stream from this type
    pub fn serialize(
        &self,
        stream: &mut WriteByteStream,
        data_value: &DataValue,
    ) -> Result<(), SerializationError> {
        use SerializationError::*;
        match (self, data_value) {
            (DataType::Int, DataValue::Int(x)) => {
                let bytes: [u8; 8] = x.to_be_bytes();
                let slice: &[u8] = &bytes;
                stream.write(slice)?;
                Ok(())
            }
            (DataType::Char(size), DataValue::Char(s)) => {
                if s.len() > *size {
                    Err(StringOverflow(*size, s.len()))
                } else {
                    stream.write(s.as_bytes())?;
                    stream.pad(*size - s.len())?;
                    Ok(())
                }
            }
            (data_type, data_value) => Err(TypeMismatch(data_type.clone(), data_value.clone())),
        }
    }

    /// Retreives the size of an element in bytes
    pub fn size(&self) -> usize {
        match self {
            Self::Char(n) => *n,
            Self::Int => 8,
        }
    }
}

impl std::fmt::Display for DataType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Char(len) => write!(f, "Char({})", len),
            Self::Int => write!(f, "Int"),
        }
    }
}

///
#[derive(Debug)]
pub struct Serializer {}

impl Serializer {
    pub fn new() -> Self {
        Self {}
    }

    pub fn serialize(
        dtypes: &Vec<DataType>,
        values: &Vec<DataValue>,
    ) -> Result<Vec<u8>, SerializationError> {
        let capacity: usize = dtypes.iter().map(|d| d.size()).sum();
        let mut write_stream = WriteByteStream::new(capacity);
        for (dtype, value) in dtypes.iter().zip(values.iter()) {
            dtype.serialize(&mut write_stream, value)?;
        }
        Ok(write_stream.buffer)
    }
}

impl Default for Serializer {
    fn default() -> Self {
        Self {}
    }
}

///
#[derive(Debug)]
pub struct Deserializer {}

impl Deserializer {
    pub fn deserialize(
        stream: &mut ReadByteStream,
        dtypes: Vec<DataType>,
    ) -> Result<Vec<DataValue>, DeserializationError> {
        dtypes.into_iter().map(|d| d.deserialize(stream)).collect()
    }

    pub fn deserialize_next(
        stream: &mut ReadByteStream,
        dtype: DataType,
    ) -> Result<DataValue, DeserializationError> {
        dtype.deserialize(stream)
    }
}

impl Default for Deserializer {
    fn default() -> Self {
        Self {}
    }
}

#[cfg(test)]
mod tests {

    use crate::serialization::{
        DataType, DataValue, Deserializer, ReadByteStream, SerializationError, Serializer,
    };

    macro_rules! test_round_trip {
        ($($val:expr),+ $(,)?; $($type:expr),+ $(,)?) => {
            {
                let values = vec![$($val),+];
                let types = vec![$($type),+];
                let serialized = Serializer::serialize(&types, &values).unwrap();

                let mut stream = ReadByteStream::new(&serialized);
                let deserialized = Deserializer::deserialize(&mut stream, types).unwrap();
                println!("{:?}", deserialized);
                assert_eq!(values, deserialized)
            }
        };
    }

    #[test]
    fn test_ints() {
        test_round_trip![DataValue::Int(5), DataValue::Int(10); DataType::Int, DataType::Int,]
    }

    #[test]
    fn test_chars() {
        test_round_trip![
            DataValue::Char("test".to_string()),
            DataValue::Char("my_char".to_string());
            DataType::Char(10),
            DataType::Char(10),
        ]
    }

    #[test]
    fn test_mix() {
        test_round_trip![
            DataValue::Int(5),
            DataValue::Char("test".to_string()),
            DataValue::Int(10),
            DataValue::Char("my_char".to_string());
            DataType::Int,
            DataType::Char(10),
            DataType::Int,
            DataType::Char(7),
        ]
    }

    #[test]
    fn raises_string_overflow() {
        let res = Serializer::serialize(
            &vec![DataType::Char(3)],
            &vec![DataValue::Char("overflow".to_string())],
        );
        assert_eq!(res, Err(SerializationError::StringOverflow(3, 8)))
    }

    #[test]
    fn to_rust_type() {
        let values = vec![DataValue::Int(5)];
        let types = vec![DataType::Int];
        let serialized = Serializer::serialize(&types, &values).unwrap();

        let mut stream = ReadByteStream::new(&serialized);
        to_rust_type!(stream, DataType::Int, DataValue::Int(x));
        assert_eq!(x, 5)
    }
}
