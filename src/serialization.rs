//! Module for serializing / deserializing rows

/// Enum to represent possible dtypes in a table
#[derive(Debug)]
pub enum DataType {
    Varchar(usize),
    Int,
}

/// Enum to represent Rust values from table dtypes
#[derive(Debug)]
pub enum DataValue {
    Varchar(String),
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
enum DeserializeError {
    #[error("Error deserializing string: {0}")]
    StringDeserializeError(String),
    #[error("Error deserializing int: {0}")]
    IntDeserializeError(String),
}

impl DataType {
    /// deserialize a datatype byte stream to a DataValue
    fn deserialize(self, bytes: &mut ByteStream) -> Result<DataValue, DeserializeError> {
        use DeserializeError::*;
        match self {
            Self::Varchar(x) => Ok(DataValue::Varchar(
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
    fn serialize(self, buffer: &mut Vec<u8>) -> Result<(), String> {
        match self {
            Self::Int(x) => {
                let bytes: [u8; 8] = x.to_be_bytes();
                let slice: &[u8] = &bytes;
                buffer.extend_from_slice(slice);
                Ok(())
            }
            Self::Varchar(s) => {
                buffer.extend_from_slice(s.as_bytes());
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    macro_rules! test_dummy {
        () => {};
    }

    fn test_string() {}
}
