//! BTrees
use super::buffer_pool::{BufferPool, DBReader, PageRef, BufferPoolError};
use super::serialization::{Serializer, DataType, DataValue, Deserializer, ReadByteStream, to_rust_type};
use std::sync::Arc;

struct Header {
    header_size: i64,
    page_id: i64,
    page_type: i64,
    free_space: i64,
    log_seq: i64,
    item_count: i64,
    free_space_ptr: i64,
    left_ptr: i64,
    right_ptr: i64,
}

impl Header {
    fn new(header: [u8; 72]) -> Self {
        let mut stream = ReadByteStream::new(&header);
        to_rust_type!(stream, DataType::Int, DataValue::Int(page_id));
        to_rust_type!(stream, DataType::Int, DataValue::Int(page_type));
        to_rust_type!(stream, DataType::Int, DataValue::Int(header_size));
        to_rust_type!(stream, DataType::Int, DataValue::Int(free_space));
        to_rust_type!(stream, DataType::Int, DataValue::Int(log_seq));
        to_rust_type!(stream, DataType::Int, DataValue::Int(item_count));
        to_rust_type!(stream, DataType::Int, DataValue::Int(free_space_ptr));
        to_rust_type!(stream, DataType::Int, DataValue::Int(left_ptr));
        to_rust_type!(stream, DataType::Int, DataValue::Int(right_ptr));
        Self {
            header_size,
            page_id,
            page_type,
            free_space,
            log_seq,
            item_count,
            free_space_ptr,
            left_ptr,
            right_ptr
        }
    }
}



enum PageInterpreter {
    Schema, 
    Node, 
    TupleLeaf,
    IndexLeaf
}

impl PageInterpreter {
    fn init(&self, page_id: u64, bytes: &mut [u8]) {
        let page_size = bytes.len();
        let page_type = match self {
            Self::Schema => 0,
            Self::Node => 1, 
            Self::TupleLeaf => 2,
            Self::IndexLeaf => 3
        };
        let header = self.new_header(page_id, page_type, page_size);
        bytes[0..header.len()].copy_from_slice(&header);
    }

    /// Init header with a few things: 
    /// 1. Header size (int)
    /// 2. Page ID (int)
    /// 3. Page type (int)
    /// 4. free space (int)
    /// 5. log sequence number (not current used) (int)
    /// 6. item count (int)
    /// 7. free space pointer (int)
    /// 8. sibling pointer left (int)
    /// 9. sibling pointer right (int)
    fn new_header(&self, page_id: u64, page_type: u64, page_size: usize) -> Vec<u8> {
        let dtypes = vec![DataType::Int; 9];
        let header_size: usize = dtypes.iter().map(|t| t.size()).sum();
        let values = vec![
            DataValue::Int(page_id as i64),
            DataValue::Int(page_type as i64),
            DataValue::Int(header_size as i64),
            DataValue::Int( (page_size - header_size) as i64),
            DataValue::Int(0),
            DataValue::Int(0),
            DataValue::Int(header_size as i64),
            DataValue::Int(0 as i64),
            DataValue::Int(0 as i64),
        ];
        Serializer::serialize(&dtypes, &values).unwrap()
    }

}

struct BTree<R: DBReader> {
    pool: Arc<BufferPool<R>>,
}

impl<R: DBReader> BTree<R> {
    fn get_interpreter(&self, page: PageRef<R>) -> PageInterpreter {
        let page_lock = page.read().unwrap();
        match self.get_page_type(&page_lock) {
            0 => PageInterpreter::Schema,
            _ => // etc
        }
    }

    fn get_page_type(&self, bytes: &[u8]) -> u8 {
        bytes[0]
    }
}
