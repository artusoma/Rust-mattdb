//!
//! Every type of page supports 3 operations:
//! insert
//! delete
//! next
//!

use crate::buffer_pool::{BufferPool, DBReader, Page, PageRef};
use crate::serialization::ReadByteStream;
use crate::to_rust_type;

use super::buffer_pool::PageID;
use super::serialization::{DataType, DataValue, Serializer};
use std::sync::{Arc, RwLockReadGuard};

use super::reader::{HeaderElem, PageReader, PageType, PageWriter};

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

struct Tuple<'a>(&'a [u8]);

/// Tuple structure for leaf page tuples
#[derive(Debug, PartialEq)]
struct Tuple<'a> {
    size: u16,
    key_size: u16,
    key: &'a [u8],
    value: &'a [u8]
}


impl<'a> Tuple<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        todo!()
    }
}

/// ScanIterator returns tuples
struct ScanIterator<'a, R: DBReader> {
    pool: Arc<BufferPool<R>>,
    page: PageRef<R>,
    end_key: &'a [u8],
    idx: usize,
}

impl<'a, R: DBReader> ScanIterator<'a, R> {
    fn new(pool: Arc<BufferPool<R>>, page: PageRef<R>, end_key: &'a [u8], idx: usize) -> Self {
        Self {
            pool,
            page,
            end_key,
            idx,
        }
    }
}

impl<'a, R: DBReader> std::iter::Iterator for ScanIterator<'a, R> {
    type Item = Vec<u8>;

    fn next(&mut self) -> Option<Self::Item> {
        // Get read lock for page to check the header
        let lock = self.page.read().unwrap();
        let item_count = lock.get_header(HeaderElem::ItemCount) as usize;
        let ptr = lock.get_header(HeaderElem::RightPtr) as u64;
        drop(lock);

        if self.idx >= item_count {
            self.page = self.pool.get_page_ref(ptr as u64).unwrap();
            self.idx = 0;
        }

        let lock = self.page.read().unwrap();

        // Get next pair. Check if we are at end.
        let (key, value) = lock.tuple_unsafe(self.idx);
        self.idx += 1;
        if key > self.end_key {
            None
        } else {
            Some(value.to_vec())
        }
    }
}

#[derive(Debug)]
struct BTree<R: DBReader> {
    pool: Arc<BufferPool<R>>,
}

impl<R: DBReader> BTree<R> {
    /// Return an iterator to iterate over tuples in leaf nodes, using sibling pointers to 
    /// move laterally
    fn iter_scan<'a>(
        &'a self,
        page_root: PageID,
        start: &'a [u8],
        end: &'a [u8],
    ) -> ScanIterator<'a, R> {
        let page = self.pool.get_page_ref(page_root).unwrap();
        let (leaf, _) = self.get_leaf(page, start, Vec::new());

        // Get start index of search in page
        let start_idx = {
            let lock = leaf.read().unwrap();
            lock.find_key(start, 0, lock.get_header(HeaderElem::ItemCount) as usize)
                .unwrap()
        };
        ScanIterator::new(Arc::clone(&self.pool), leaf, end, start_idx)
    }

    fn insert_recurs(&self, leaf: PageRef<R>, key: &[u8], value: &[u8], mut parents: Vec<PageID>) {
        // If leaf has room, insert. Else, split and insert into parent
        let room = leaf.get(HeaderElem::FreeSpace) as usize;
        let key_size: usize = key.iter().map(|t| t.size()).sum();
        let value_size: usize = value.iter().map(|t| t.size()).sum();

        if key_size + value_size + 2usize > room {
            // Split and insert into parent
            let parent_id = match parents.pop() {
                Some(parent_id) => parent_id,
                None => {
                    // Create new page
                    let new_id = self.pool.new_page();
                    PageContent::init(&mut self, new_id, PageType::Node);

                    new_id
                }
            };

            // Split page
            todo!();

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

    fn insert_tuple(&self, page_root: PageID, key: &[u8], value: &[u8]) {
        // Get leaf page if not leaf page
        let (leaf, parents) =
            self.get_leaf(self.pool.get_page_ref(page_root).unwrap(), key, Vec::new());

        // Call insert page which will be recursive
        self.insert_recurs(leaf, key, value, parents);
    }

    /// Get the leaf node of the tree containing the given key
    fn get_leaf<'a>(
        &'a self,
        page: PageRef<R>,
        key: &[u8],
        mut parents: Vec<PageID>,
    ) -> (PageRef<R>, Vec<PageID>) {
        let page_type = {
            let lock = page.read().unwrap();
            PageType::new(lock.get_header(HeaderElem::PageType).into())
        };
        match page_type {
            PageType::Leaf => (page, parents),
            PageType::Node => {
                let (next_id, this_id) = {
                    let lock = page.read().unwrap();
                    (
                        lock.get_header(HeaderElem::RightPtr),
                        lock.get_header(HeaderElem::PageID),
                    )
                };
                parents.push(this_id.into());
                self.get_leaf(
                    self.pool.get_page_ref(next_id.into()).unwrap(),
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
