//!
//! Every type of page supports 3 operations:
//! insert
//! delete
//! next
//!

use crate::buffer_pool::{BufferPool, DBReader};

use super::buffer_pool::PageID;
use super::serialization::{DataType, DataValue};
use std::iter::Scan;
use std::sync::Arc;

const HEADER_SIZE: usize = 64usize;

#[derive(Debug, PartialEq)]
enum PageType {
    /// Nodes always map to Key => PageID
    Node,
    /// Leaf always maps Key => Tuple
    Leaf,
}

#[derive(Debug, PartialEq)]
enum BTreeValue {
    PageID(PageID),
    Tuple(Vec<DataValue>),
    NextPage(PageID),
    Done,
}

#[derive(Debug, PartialEq)]
enum Sibling {
    Right,
    Left,
}

/// Single source of truth for interpreting bytes of a page
#[derive(Debug)]
struct BTreePage<'a> {
    bytes: &'a [u8],
    key_type: Vec<DataType>,
    value_type: Vec<DataType>,
}

impl<'a> BTreePage<'a> {
    fn new(bytes: &'a [u8], key_type: Vec<DataType>, value_type: Vec<DataType>) -> Self {
        Self {
            bytes,
            key_type,
            value_type,
        }
    }

    fn get_type(&self) -> PageType {
        todo!()
    }

    fn free_space(&self) -> usize {
        todo!()
    }

    fn insert(&self, key: &DataValue, value: &DataValue) {
        todo!()
    }

    fn delete(&self) {
        todo!()
    }

    fn next(&self, start: &Vec<DataValue>, end: &Vec<DataValue>) -> BTreeValue {
        match self.get_type() {
            PageType::Node => {
                // Returns the page id of the child
                todo!()
            }
            PageType::Leaf => {
                // Can return a tuple, done if the bounds are exhaused, or request the next page
                todo!()
            }
        }
    }

    /// Gets left or right sibling from header
    fn sibling(&self, sibling: Sibling) -> PageID {
        todo!()
    }

    /// Item count from header
    fn items() -> usize {
        todo!()
    }

    /// Looks in slot array using header size and number of items as bounds
    fn binary_search(&self, key: &Vec<DataValue>) {
        todo!()
    }
}

struct ScanIterator<'a, R: DBReader> {
    pool: Arc<BufferPool<R>>,
    page: BTreePage<'a>,
    start: Vec<DataValue>,
    end: Vec<DataValue>,
}
impl<'a, R: DBReader> std::iter::Iterator for ScanIterator<'a, R> {
    type Item = Vec<DataValue>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.page.next(&self.start, &self.end) {
                BTreeValue::Tuple(t) => return Some(t),
                BTreeValue::Done => return None,
                BTreeValue::NextPage(id) => {
                    let bytes = &**(*self.pool.get_page_ref(id).unwrap()).read().unwrap();
                    self.page = BTreePage::new(
                        bytes,
                        self.page.key_type.clone(),
                        self.page.value_type.clone(),
                    );
                }
                _ => panic!(),
            }
        }
    }
}

#[derive(Debug)]
struct BTree<R: DBReader> {
    pool: Arc<BufferPool<R>>,
}

impl<R: DBReader> BTree<R> {
    fn scan(
        &self,
        page_root: PageID,
        key_type: Vec<DataType>,
        value_type: Vec<DataType>,
        start: Vec<DataValue>,
        end: Vec<DataValue>,
    ) {
        let page_ref = self.pool.get_page_ref(page_root).unwrap();
        let lock = page_ref.read().unwrap();
        let page = BTreePage::new(&**lock, key_type, value_type);
    }
}
