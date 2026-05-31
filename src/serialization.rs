//! Module for serializing / deserializing rows

/// Enum to represent possible dtypes in a table
#[derive(Debug, PartialEq, Clone)]
pub enum DataType {
    Char(usize),
    Int,
}

/// Enum to represent Rust values from table dtypes
#[derive(Debug, PartialEq, Clone)]
pub enum DataValue {
    Char(String),
    Int(u64),
}

/// Stream of u8 slice
/// We actually don't want this to iter because we don't want to copy
#[derive(Debug)]
struct ByteStream<'a> {
    bytes: &'a [u8],
    position: usize,
    length: usize,
}

impl<'a> ByteStream<'a> {
    /// Get next `len` bytes from data and advance the position
    fn next(&mut self, len: usize) -> &[u8] {
        let start = self.position;
        let end = start + len;
        self.position += len;
        &self.bytes[start..end]
    }
}

#[derive(thiserror::Error, Debug)]
enum DeserializationError {
    #[error("Error deserializing string: {0}")]
    StringDeserializeError(String),
    #[error("Error deserializing int: {0}")]
    IntDeserializeError(String),
}

#[derive(thiserror::Error, Debug)]
enum SerializtionError {
    #[error("Error serializing: {0}")]
    SerializtionError(String),
}

impl DataType {
    /// deserialize a datatype byte stream to a DataValue
    fn deserialize(self, bytes: &mut ByteStream) -> Result<DataValue, DeserializationError> {
        use DeserializationError::*;
        match self {
            Self::Char(x) => Ok(DataValue::Char(
                str::from_utf8(bytes.next(x))
                    .map_err(|e| StringDeserializeError(e.to_string()))?
                    .to_owned(),
            )),
            Self::Int => Ok(DataValue::Int(u64::from_be_bytes(
                bytes
                    .next(8)
                    .try_into()
                    .map_err(|e: std::array::TryFromSliceError| {
                        IntDeserializeError(e.to_string())
                    })?,
            ))),
        }
    }
}

impl DataValue {
    /// Serialize a Rust value to a byte stream DataType
    fn serialize(self, buffer: &mut Vec<u8>) -> Result<(), SerializtionError> {
        match self {
            Self::Int(x) => {
                let bytes: [u8; 8] = x.to_be_bytes();
                let slice: &[u8] = &bytes;
                buffer.extend_from_slice(slice);
                Ok(())
            }
            Self::Char(s) => {
                buffer.extend_from_slice(s.as_bytes());
                Ok(())
            }
        }
    }

    fn to_type(&self) -> DataType {
        match self {
            Self::Int(_) => DataType::Int,
            Self::Char(x) => DataType::Char(x.len()),
        }
    }
}

///
#[derive(Debug)]
pub struct Serializer {}

impl Serializer {
    fn serialize(
        &self,
        buffer: &mut Vec<u8>,
        values: Vec<DataValue>,
    ) -> Result<(), SerializtionError> {
        for value in values.into_iter() {
            value.serialize(buffer)?
        }
        Ok(())
    }
}

///
#[derive(Debug)]
pub struct Deserializer {}

impl Deserializer {
    fn deserialize(
        &self,
        stream: &mut ByteStream,
        dtypes: Vec<DataType>,
    ) -> Result<Vec<DataValue>, DeserializationError> {
        dtypes.into_iter().map(|d| d.deserialize(stream)).collect()
    }
}

#[cfg(test)]
mod tests {

    use crate::serialization::{ByteStream, DataType, DataValue, Deserializer, Serializer};

    macro_rules! test_round_trip {
        ($($val:expr),+ $(,)?) => {
            {
                let values = vec![$($val),+];
                let mut buffer = Vec::<u8>::new();
                let _ = Serializer {}.serialize(&mut buffer, values.clone()).unwrap();

                let mut stream = ByteStream {
                    bytes: &buffer,
                    position: 0,
                    length: 0,
                };
                let dtypes = values.iter().map(|v| v.to_type()).collect();
                let deserialized = Deserializer {}.deserialize(&mut stream, dtypes).unwrap();
                println!("{:?}", deserialized);
                assert_eq!(values, deserialized)
            }
        };
    }

    #[test]
    fn test_ints() {
        test_round_trip![DataValue::Int(5), DataValue::Int(10)]
    }

    #[test]
    fn test_chars() {
        test_round_trip![
            DataValue::Char("test".to_string()),
            DataValue::Char("my_char".to_string())
        ]
    }

    #[test]
    fn test_mix() {
        test_round_trip![
            DataValue::Int(5),
            DataValue::Char("test".to_string()),
            DataValue::Int(10),
            DataValue::Char("my_char".to_string())
        ]
    }
}
