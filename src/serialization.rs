pub enum DataType {
    Varchar(usize),
    Int,
}

pub enum DataValue {
    Varchar(String),
    Int(u64),
}

struct ByteStream<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> ByteStream<'a> {
    fn next(&mut self, len: usize) -> &[u8] {
        let start = self.position;
        let end = start + len;
        self.position += len;
        &self.bytes[start..end]
    }
}

impl DataType {
    fn deserialize(self, bytes: &mut ByteStream) -> DataValue {
        match self {
            DataType::Varchar(x) => {
                DataValue::Varchar(str::from_utf8(bytes.next(x)).unwrap().to_owned())
            }
            Self::Int => {
                let b: [u8; 8] = bytes.next(8).try_into().unwrap();
                DataValue::Int(u64::from_be_bytes(b))
            },
        }
    }
}

impl DataValue {
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
