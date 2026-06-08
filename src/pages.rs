//! BTree
//!
//! BTree:
//! Only knows how to take a page root and manipulate a tree with delete and
//! insert, or return values from a scan
//! BTree uses PageRw. The idea is that PageRw abstracts whether
//! it is reading in bytes, or reading fully into memory.
//!     scan(
//!         page_root: PageID,
//!         key_type: Vec<DataType>,
//!         value_type: Vec<DataType>
//!         start_key: Vec<DataValue>,
//!         end_key: Vec<DataValue>,
//!     ) -> ScanIterator
//!     delete(page_root: PageID, key: Vec<DataValue>) -> Result
//!     insert(
//!         page_root: PageID,
//!         key: Vec<DataValue>,
//!         data: Vec<DataValue>
//!     ) -> Result
//!
//!
//! For example, on an insert, the BTree will ask PageRw.
//! PageRw will return a status enum that is either Ok or NeedsSplit.
//!
//! On a scan:
//!    First, ScanIterator initializes by traversing down to a leaf node:
//!         Loop match next_page_type {}, calling PageRw to get next page
//!    Then, it will call continually call next method on PageRw.
//!         /// pageRw next() -> enum::{Data, Done, NeedNextSibling}

use crate::buffer_pool::Page;

use super::buffer_pool::{BufferPool, BufferPoolError, DBReader, PageID, PageRef};
use super::serialization::{
    DataType, DataValue, Deserializer, ReadByteStream, Serializer, to_rust_type,
};
use std::sync::Arc;

use thiserror::Error;

type Types = Vec<DataType>;
type Values = Vec<DataValue>;

#[derive(Debug, Error)]
pub enum BTreeError {
    #[error("Key not found")]
    KeyNotFound,
}

enum InsertResult {
    Ok,
    OutOfSpace,
}

#[derive(Debug)]
enum LeafReadResult {
    Tuple(Values),
    PageNeeded(PageID),
}

#[derive(Debug, PartialEq)]
struct Node;

impl Node {
    fn get_child<R: DBReader>(&self, page: &PageRef<R>, key: &Values, key_type: &Types) -> PageID {
        todo!()
    }
}

#[derive(Debug, PartialEq)]
enum Leaf {
    IndexLeaf,
    DataLeaf,
}

impl Leaf {
    fn next<R: DBReader>(
        &self,
        page: &PageRef<R>,
        key: &Values,
        key_type: &Types,
        value_types: &Types,
    ) -> LeafReadResult {
        match self {
            Self::DataLeaf => {
                todo!()
            }
            Self::IndexLeaf => {
                todo!()
            }
        }
    }

    fn insert<R: DBReader>(
        &self,
        page: &PageRef<R>,
        key: &Values,
        key_type: &Types,
        value_types: &Types,
        values: &Values,
    ) -> InsertResult {
        todo!()
    }

    fn delete<R: DBReader>(
        &self,
        page: &PageRef<R>,
        key: &Values,
        key_type: &Types,
    ) -> InsertResult {
        todo!()
    }
}

#[derive(Debug, PartialEq)]
enum PageType {
    Node(Node),
    Leaf(Leaf),
}

struct BTree<R: DBReader> {
    pool: Arc<BufferPool<R>>,
    key_type: Types,
    value_type: Types,
}

fn get_page_type<R: DBReader>(page_ref: &PageRef<R>) -> PageType {
    todo!()
}

fn get_next_sibling<R: DBReader>(page_ref: &PageRef<R>) -> PageID {
    todo!()
}

impl<R: DBReader> BTree<R> {
    fn scan<'a>(
        &self,
        page_id: PageID,
        start_key: &'a Values,
        end_key: &'a Values,
    ) -> ScanIterator<'a, R> {
        // Find leaf node
        let leaf_id = self.get_leaf(page_id, start_key);
        let page_ref = self.pool.get_page_ref(leaf_id).unwrap();
        let page_type = get_page_type(&page_ref);
        // Return an iterator to get rows in scan
        ScanIterator::new(
            page_ref,
            page_type,
            &start_key,
            &end_key,
            Arc::clone(&self.pool),
        )
    }

    fn get_leaf(&self, page_id: PageID, key: &Values) -> PageID {
        let mut page = page_id;
        let mut page_ref = self.pool.get_page_ref(page).unwrap();
        let mut page_type = get_page_type(&page_ref);
        loop {
            if let PageType::Node(node) = page_type {
                page = node.get_child(&page_ref, key, &self.key_type);
                page_ref = self.pool.get_page_ref(page).unwrap();
                page_type = get_page_type(&page_ref);
            } else {
                break page;
            }
        }
    }

    fn insert(&self, page_id: PageID, key: &Vec<DataValue>, value: &Vec<DataValue>) {
        let leaf_id = self.get_leaf(page_id, key);
        let page_ref = self.pool.get_page_ref(leaf_id).unwrap();
        let page_type = get_page_type(&page_ref);
        match page_type {
            PageType::Leaf(leaf) => match leaf.insert(&page_ref, key, &self.key_type, &self.value_type, value) {
                InsertResult::Ok => return,
                InsertResult::OutOfSpace => {
                    todo!()
                }
            },
            _ => panic!(),
        }
    }

    fn delete(&self, key: Vec<DataValue>) -> Result<(), BTreeError> {
        todo!()
    }
}

struct ScanIterator<'a, R: DBReader> {
    leaf: PageRef<R>,
    page_type: Leaf,
    start_key: &'a Values,
    end_key: &'a Values,
    pool: Arc<BufferPool<R>>,
}

impl<'a, R: DBReader> ScanIterator<'a, R> {
    fn new(
        leaf: PageRef<R>,
        page_type: Leaf,
        start_key: &'a Values,
        end_key: &'a Values,
        pool: Arc<BufferPool<R>>,
    ) -> Self {
        Self {
            leaf,
            page_type,
            start_key,
            end_key,
            pool,
        }
    }
}

impl<R: DBReader> std::iter::Iterator for ScanIterator<'_, R> {
    type Item = Vec<DataValue>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.page_type.next(&self.leaf, self.param) {
                LeafReadResult::Tuple(tuple) => return Some(tuple),
                LeafReadResult::PageNeeded(page_id) => {
                    self.leaf = self.pool.get_page_ref(page_id).unwrap();
                }
            }
        }
    }
}

// OLD CODE BELOW

// struct Header {
//     header_size: i64,
//     page_id: i64,
//     page_type: i64,
//     free_space: i64,
//     log_seq: i64,
//     item_count: i64,
//     free_space_ptr: i64,
//     left_ptr: i64,
//     right_ptr: i64,
// }

// impl Header {
//     fn new(header: [u8; 72]) -> Self {
//         let mut stream = ReadByteStream::new(&header);
//         to_rust_type!(stream, DataType::Int, DataValue::Int(page_id));
//         to_rust_type!(stream, DataType::Int, DataValue::Int(page_type));
//         to_rust_type!(stream, DataType::Int, DataValue::Int(header_size));
//         to_rust_type!(stream, DataType::Int, DataValue::Int(free_space));
//         to_rust_type!(stream, DataType::Int, DataValue::Int(log_seq));
//         to_rust_type!(stream, DataType::Int, DataValue::Int(item_count));
//         to_rust_type!(stream, DataType::Int, DataValue::Int(free_space_ptr));
//         to_rust_type!(stream, DataType::Int, DataValue::Int(left_ptr));
//         to_rust_type!(stream, DataType::Int, DataValue::Int(right_ptr));
//         Self {
//             header_size,
//             page_id,
//             page_type,
//             free_space,
//             log_seq,
//             item_count,
//             free_space_ptr,
//             left_ptr,
//             right_ptr,
//         }
//     }
// }

// enum PageInterpreter {
//     Schema,
//     Node,
//     TupleLeaf,
//     IndexLeaf,
// }

// impl PageInterpreter {
//     fn init(&self, page_id: u64, bytes: &mut [u8]) {
//         let page_size = bytes.len();
//         let page_type = match self {
//             Self::Schema => 0,
//             Self::Node => 1,
//             Self::TupleLeaf => 2,
//             Self::IndexLeaf => 3,
//         };
//         let header = self.new_header(page_id, page_type, page_size);
//         bytes[0..header.len()].copy_from_slice(&header);
//     }

//     /// Init header with a few things:
//     /// 1. Header size (int)
//     /// 2. Page ID (int)
//     /// 3. Page type (int)
//     /// 4. free space (int)
//     /// 5. log sequence number (not current used) (int)
//     /// 6. item count (int)
//     /// 7. free space pointer (int)
//     /// 8. sibling pointer left (int)
//     /// 9. sibling pointer right (int)
//     fn new_header(&self, page_id: u64, page_type: u64, page_size: usize) -> Vec<u8> {
//         let dtypes = vec![DataType::Int; 9];
//         let header_size: usize = dtypes.iter().map(|t| t.size()).sum();
//         let values = vec![
//             DataValue::Int(page_id as i64),
//             DataValue::Int(page_type as i64),
//             DataValue::Int(header_size as i64),
//             DataValue::Int((page_size - header_size) as i64),
//             DataValue::Int(0),
//             DataValue::Int(0),
//             DataValue::Int(header_size as i64),
//             DataValue::Int(0 as i64),
//             DataValue::Int(0 as i64),
//         ];
//         Serializer::serialize(&dtypes, &values).unwrap()
//     }

//     fn get_header(&self) -> Header {
//         Header::new(header)
//     }
// }

// struct BTree<R: DBReader> {
//     pool: Arc<BufferPool<R>>,
// }

// impl<R: DBReader> BTree<R> {
//     fn get_interpreter(&self, page: PageRef<R>) -> PageInterpreter {
//         let page_lock = page.read().unwrap();
//         match self.get_page_type(&page_lock) {
//             0 => PageInterpreter::Schema,
//             1 => PageInterpreter::Node,
//             2 => PageInterpreter::IndexLeaf,
//             3 => PageInterpreter::TupleLeaf,
//             _ => panic!(),
//         }
//     }

//     fn get_page_type(&self, bytes: &[u8]) -> i64 {
//         let mut stream = ReadByteStream::new(&bytes[12..16]);
//         to_rust_type!(stream, DataType::Int, DataValue::Int(page_type));
//         page_type
//     }
// }
