//!
//! Every type of page supports 3 operations:
//! insert
//! delete
//! next
//!

use crate::buffer_pool::{BufferPool, DBReader, Page};
use crate::serialization::{Deserializer, ReadByteStream};
use crate::to_rust_type;

use super::buffer_pool::{PAGE_SIZE, PageID};
use super::serialization::{DataType, DataValue, Serializer};
use std::iter::Scan;
use std::net::Shutdown::Read;
use std::sync::Arc;

/// Page type in the BTree. Node pages map keys to page ids, leaf pages map keys to tuples
#[derive(Debug, PartialEq)]
enum PageType {
    /// Nodes always map to Key => PageID
    Node,
    /// Leaf always maps Key => Tuple
    Leaf,
}

impl PageType {
    fn id(&self) -> u64 {
        match self {
            Self::Node => 0,
            Self::Leaf => 1,
        }
    }

    fn new(type_id: u64) -> Self {
        match type_id {
            0 => Self::Node,
            1 => Self::Leaf,
            _ => panic!(),
        }
    }
}

enum HeaderElem {
    PageID,
    PageType,
    FreeSpace,
    LastCommit,
    ItemCount,
    HeaderSize,
    LeftPtr,
    RightPtr,
    FreeSpacePtr,
}

impl HeaderElem {
    fn offset(&self) -> usize {
        match self {
            Self::PageID => 0,
            Self::PageType => 4,
            Self::FreeSpace => 8,
            Self::LastCommit => 12,
            Self::ItemCount => 16,
            Self::HeaderSize => 20,
            Self::LeftPtr => 24,
            Self::RightPtr => 28,
            Self::FreeSpacePtr => 32,
        }
    }
}

const HEADER_SIZE: usize = 4 * 9;

struct PageContent<'a> {
    header: Vec<u8>,
    body: Vec<u8>,
    key_type: &'a [DataType],
    value_type: &'a [DataType],
}

impl<'a> PageContent<'a> {
    fn new(mut bytes: Vec<u8>, key_type: &'a [DataType], value_type: &'a [DataType]) -> Self {
        let body = bytes.split_off(HEADER_SIZE);
        Self {
            header: bytes,
            body,
            key_type,
            value_type,
        }
    }

    /// Inserts a new slot, returning a pointer to the open space
    fn insert_slot(&self, data_size: usize) -> usize {
        todo!()
    }

    /// Inserts tuple
    fn insert_data(&self, bytes: &[u8]) -> usize {
        todo!()
    }

    fn get(&self, element: HeaderElem) -> u64 {
        let offset = element.offset();
        let mut stream = ReadByteStream::new(&self.header[offset..offset + 4]);
        to_rust_type!(stream, DataType::Int, DataValue::Int(value));
        value as u64
    }

    fn set(&mut self, element: HeaderElem, value: u64) {
        let offset = element.offset();
        self.header[offset..offset + 4].copy_from_slice(
            Serializer::serialize_single(&DataType::Int, &DataValue::Int(value as i32))
                .unwrap()
                .as_slice(),
        );
    }

    /// Looks in slot array using header size and number of items as bounds
    fn find_key_idx(&self, target_key: &[DataValue], low: usize, high: usize) -> Option<usize> {
        // Initize index to middle of slot array
        let idx = (high - low) / 2 + low;
        let key = self.element(idx).0;

        if key == *target_key {
            return Some(idx);
        }

        if low == high {
            return None;
        }

        if key.as_slice() < target_key {
            self.find_key_idx(target_key, idx + 1, high)
        } else {
            if idx == 0 {
                return None;
            } else {
                self.find_key_idx(target_key, low, idx - 1)
            }
        }
    }

    /// Find child
    fn find_child(&self, target_key: &[DataValue], low: usize, high: usize) -> Option<usize> {
        todo!()
    }

    /// Returns a Tuple from a given slot
    fn element(&self, idx: usize) -> (Vec<DataValue>, Vec<DataValue>) {
        let mut stream = ReadByteStream::new(&self.body[self.get_tuple_ptr(idx)..self.body.len()]);
        (
            stream.next(self.key_type).unwrap(),
            stream.next(self.value_type).unwrap(),
        )
    }

    /// Returns the tuple offset of the slot at idx
    ///
    /// Slots are 16 bytes in size (i16; small int)
    fn get_tuple_ptr(&self, idx: usize) -> usize {
        let mut slot_stream = ReadByteStream::new(&self.body[2usize * idx..2usize * idx + 2usize]);
        to_rust_type!(
            slot_stream,
            DataType::SmallInt,
            DataValue::SmallInt(tuple_offset)
        );
        tuple_offset as usize
    }

    fn page_type(&self) -> PageType {
        PageType::new(self.get(HeaderElem::PageType))
    }
}

/// Header to tuples to store metadata about the tuple
#[derive(Debug, PartialEq)]
struct TupleHeader {}

impl TupleHeader {
    fn size(&self) -> usize {
        0
    }

    fn to_bytes(&self) -> Vec<u8> {
        vec![]
    }

    fn new() -> Self {
        Self {}
    }
}

/// Tuple structure for leaf page tuples
#[derive(Debug, PartialEq)]
struct Tuple {
    header: TupleHeader,
    key: Vec<DataValue>,
    value: Vec<DataValue>,
}

/// ScanIterator returns tuples
struct ScanIterator<'a, R: DBReader> {
    pool: Arc<BufferPool<R>>,
    page: PageContent<'a>,
    end_key: &'a [DataValue],
    idx: usize,
}

impl<'a, R: DBReader> ScanIterator<'a, R> {
    fn new(
        pool: Arc<BufferPool<R>>,
        page: PageContent<'a>,
        end_key: &'a [DataValue],
        idx: usize,
    ) -> Self {
        Self {
            pool,
            page,
            end_key,
            idx,
        }
    }
}

impl<'a, R: DBReader> std::iter::Iterator for ScanIterator<'a, R> {
    type Item = Vec<DataValue>;

    fn next(&mut self) -> Option<Self::Item> {
        // Check if we are out of room for next element. If so, fetch sibling.
        if self.idx >= self.page.get(HeaderElem::ItemCount) as usize {
            let ptr = self.page.get(HeaderElem::RightPtr);
            let bytes = (&**(*self.pool.get_page_ref(ptr).unwrap()).read().unwrap()).to_vec();
            self.page = PageContent::new(bytes, self.page.key_type, self.page.value_type);
            self.idx = 0;
        }

        // Get next pair. Check if we are at end.
        let (key, value) = self.page.element(self.idx);
        self.idx += 1;
        if key.as_slice() > self.end_key {
            None
        } else {
            Some(value)
        }
    }
}

#[derive(Debug)]
struct BTree<R: DBReader> {
    pool: Arc<BufferPool<R>>,
}

impl<R: DBReader> BTree<R> {
    fn get_content<'a>(
        &self,
        page_id: PageID,
        key_type: &'a [DataType],
        value_type: &'a [DataType],
    ) -> PageContent<'a> {
        // Get page and release lock
        let page_ref = self.pool.get_page_ref(page_id).unwrap();
        let lock = page_ref.read().unwrap();
        PageContent::new((&**lock).to_vec(), key_type, value_type)
    }

    fn scan<'a>(
        &'a self,
        page_root: PageID,
        key_type: &'a [DataType],
        value_type: &'a [DataType],
        start: &'a [DataValue],
        end: &'a [DataValue],
    ) -> ScanIterator<'a, R> {
        // Get leaf page if not leaf page
        let page = self.get_content(page_root, key_type, value_type);
        let (leaf, _) = self.get_leaf(page, start, Vec::new());
        let start_idx = leaf
            .find_key_idx(start, 0, leaf.get(HeaderElem::ItemCount) as usize)
            .unwrap();
        ScanIterator::new(Arc::clone(&self.pool), leaf, end, start_idx)
    }

    fn insert_recurs(
        &self,
        leaf: PageContent,
        key: &[DataValue],
        value: &[DataValue],
        mut parents: Vec<PageID>,
    ) {
        // If leaf has room, insert. Else, split and insert into parent
        let room = leaf.get(HeaderElem::FreeSpace) as usize;
        let key_size: usize = key.iter().map(|t| t.size()).sum();
        let value_size: usize = value.iter().map(|t| t.size()).sum();

        if key_size + value_size + 2usize > room {
            // Split and insert into parent
            let Some(parent_id) = parents.pop() else {
                // create new page
                todo!()
            };

            // Splid page
            todo!()
            
            let parent = self.get_content(parent_id, &[DataType::Int], &[DataType::Int]);
            self.insert_recurs(parent, key, value, parents);
        }

        // Serialize and hand off
        let key_bytes = Serializer::serialize(leaf.key_type, key).unwrap();
        let value_bytes = Serializer::serialize(leaf.value_type, value).unwrap();
        let mut bytes = key_bytes;
        bytes.extend(value_bytes);
        leaf.insert_data(&bytes);
    }

    fn insert(
        &self,
        page_root: PageID,
        key: &[DataValue],
        value: &[DataValue],
        key_type: &[DataType],
        value_type: &[DataType],
    ) {
        // Get leaf page if not leaf page
        let page = self.get_content(page_root, key_type, value_type);
        let (leaf, parents) = self.get_leaf(page, key, Vec::new());

        // Call insert page which will be recursive
        self.insert_recurs(leaf, key, value, parents);
    }

    fn get_leaf<'a>(
        &'a self,
        node: PageContent<'a>,
        key: &[DataValue],
        mut parents: Vec<PageID>,
    ) -> (PageContent<'a>, Vec<PageID>) {
        match node.page_type() {
            PageType::Leaf => (node, parents),
            PageType::Node => {
                let Some(next_id) =
                    node.find_child(key, 0, node.get(HeaderElem::ItemCount) as usize)
                else {
                    panic!() // Node should always return a child. 
                };
                parents.push(node.get(HeaderElem::PageID));
                self.get_leaf(
                    self.get_content(next_id as u64, node.key_type, node.value_type),
                    key,
                    parents,
                )
            }
        }
    }

    fn delete(&self) {
        todo!()
    }
}
