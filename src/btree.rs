//!
//! Every type of page supports 3 operations:
//! insert
//! delete
//! next
//!

use crate::buffer_pool::{BufferPool, DBReader, PAGE_SIZE, PageID, PageRef};
use crate::representations::page::{HeaderElem, InnerNode, Leaf, PageType, SlottedPage};
use crate::representations::tuple::Tuple;
use std::sync::Arc;

/// ScanIterator returns tuples
#[derive(Debug)]
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
        let leaf = Leaf::from_bytes(&lock);
        let item_count = leaf.get_header(HeaderElem::ItemCount) as usize;
        let ptr = leaf.get_header(HeaderElem::RightPtr);
        drop(lock);

        // If we are at end of leaf, grab next page and reset idx
        if self.idx >= item_count {
            self.page = self.pool.get_page_ref(ptr).unwrap();
            self.idx = 0;
        }

        // Retake lock with new page
        let lock = self.page.read().unwrap();
        let leaf = Leaf::from_bytes(&lock);

        // Get next pair. Check key to see if we are at end.
        let t = leaf.tuple(self.idx).unwrap();
        self.idx += 1;
        if t.key() > self.end_key {
            None
        } else {
            Some(t.value().to_vec())
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
            Leaf::from_bytes(&lock).find_key(start).unwrap()
        };
        ScanIterator::new(Arc::clone(&self.pool), leaf, end, start_idx)
    }

    fn insert_recurs(&self, leaf: PageRef<R>, tuple: &Tuple, mut parents: Vec<PageID>) {
        // The required space is the size of the tuple plus the 2 byte slot ptr.
        // Check if we have enough room in the page.
        //
        // If we don't, then we need to:
        // (1) Split the page 
        // (2) Insert a new key into the parent
        // (3) Possibly recurse that
        let room = Leaf::from_bytes(&leaf.read().unwrap()).get_header(HeaderElem::FreeSpace);
        if tuple.size() as u32 + 2u32 > room {
            // Split and insert into parent. Check if we had a parent to get the leaf. 
            // If we did, then use that as the parent id. Otherwise, we need to create a new page
            // and use that newly created page id
            let parent_id = match parents.pop() {
                Some(parent_id) => parent_id,
                None => {
                    // Create new page
                    let new_id = self.pool.new_page();
                    let new_page = self.pool.get_page_ref(new_id).unwrap();
                    {
                        InnerNode::from_bytes_mut(&mut new_page.read().unwrap()).init(new_id, PageType::Leaf, left_ptr, right_ptr, left_child_ptr);
                    };
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

    fn insert_tuple(&self, page_root: PageID, tuple: &Tuple) {
        // Get leaf page if not leaf page
        let (leaf, parents) = self.get_leaf(
            self.pool.get_page_ref(page_root).unwrap(),
            tuple.key(),
            Vec::new(),
        );

        // Call insert page which will be recursive
        self.insert_recurs(leaf, tuple, parents);
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
            PageType::new(SlottedPage::from_bytes(&lock).get_header(HeaderElem::PageType))
        };
        match page_type {
            PageType::Leaf => (page, parents),
            PageType::Node => {
                let (next_id, this_id) = {
                    let lock = page.read().unwrap();
                    let repr = SlottedPage::from_bytes(&lock);
                    (
                        repr.get_header(HeaderElem::RightPtr),
                        repr.get_header(HeaderElem::PageID),
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
