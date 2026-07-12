use crate::buffer_pool::{BufferPool, ObjectID, PageID, PageRef};
use crate::representations::page::{
    HeaderElem, InnerNode, Leaf, NULL_PTR, PageReadWriteError, PageType, SlottedPage,
};
use crate::representations::tuple::{Tuple, TupleBuf};
use crate::storage::DBReader;
use std::sync::Arc;

/// Lazy range scan iterator over a B-tree's leaf node chain.
///
/// Iterates through tuple values in leaf nodes within a key range `[start..=end]`.
/// Uses right-sibling pointers to traverse horizontally across leaf pages without
/// ascending back to inner nodes, enabling efficient sequential scans.
///
/// Tuples are fetched on-demand as the iterator advances. The scan terminates when
/// a tuple key exceeds the configured `end_key` or when the leaf chain is exhausted.
///
/// # Type Parameters
///
/// * `'a` - Lifetime of the `end_key` reference.
/// * `R` - A [`DBReader`] backing the underlying [`BufferPool`].
#[derive(Debug)]
pub struct ScanIterator<'a, R: DBReader> {
    pool: Arc<BufferPool<R>>,
    page: PageRef<R>,
    end_key: &'a [u8],
    idx: usize,
}

impl<'a, R: DBReader> ScanIterator<'a, R> {
    /// Creates a new range scan iterator.
    ///
    /// # Arguments
    ///
    /// * `pool` - The buffer pool managing page storage.
    /// * `page` - The starting leaf page for the scan.
    /// * `end_key` - The upper bound (inclusive) for the scan range.
    /// * `idx` - The starting tuple index within the initial page.
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

    /// Yields the next tuple value in the scan range.
    ///
    /// Automatically advances to the next leaf node via right-sibling pointers when
    /// the current page is exhausted. Stops when a key exceeds `end_key`.
    fn next(&mut self) -> Option<Self::Item> {
        // Get read lock for page to check the header
        let (ptr, item_count) = {
            let lock = self.page.read().unwrap();
            let leaf = Leaf::from_bytes(&lock);
            let item_count = leaf.get_header(&HeaderElem::ItemCount) as usize;
            let ptr = leaf.get_header(&HeaderElem::RightSiblingPtr);
            (ptr, item_count)
        };

        // If we are at end of leaf, grab next page and reset idx
        if self.idx >= item_count {
            if ptr == NULL_PTR {
                return None;
            }
            self.page = self.pool.get_page_ref(ptr).unwrap();
            self.idx = 0;
        }

        // Retake lock with new page
        let lock = self.page.read().unwrap();
        let leaf = Leaf::from_bytes(&lock);

        // Get next pair. Check key to see if we are at end.
        let t = leaf.tuple(self.idx).unwrap();
        self.idx += 1;
        if t.key().bytes() > self.end_key {
            None
        } else {
            Some(t.value().to_vec())
        }
    }
}

/// A B-tree index over a [`BufferPool`].
///
/// `BTree` manages page-level storage for ordered key-value data. Pages are
/// divided into two types: inner nodes that route lookups and leaf nodes that
/// store [`Tuple`] data. Leaf nodes are linked via right-sibling pointers,
/// enabling efficient range scans without ascending back to the root.
///
/// Insertions are handled recursively: when a page is full it is split and a
/// separator key is promoted to the parent, which may itself require splitting
/// up to the root.
///
/// # Type Parameters
///
/// * `R` - A [`DBReader`] that backs the [`BufferPool`] with persistent storage.
#[derive(Debug)]
pub struct BTree<R: DBReader> {
    pool: Arc<BufferPool<R>>,
}

impl<R: std::fmt::Debug + DBReader> BTree<R> {
    fn new(pool: Arc<BufferPool<R>>) -> Self {
        Self { pool }
    }

    /// Return an iterator that iterates over tuples in leaf nodes,
    /// using sibling pointers to move laterally
    pub fn iter_scan<'a>(
        &'a self,
        object_id: ObjectID,
        start: &'a [u8],
        end: &'a [u8],
    ) -> ScanIterator<'a, R> {
        let page = self
            .pool
            .get_page_ref(self.pool.get_object_root(object_id))
            .unwrap();
        let (leaf, _) = self.traverse_to_leaf(page, start, Vec::new());

        // Get start index of search in page
        let start_idx = {
            let lock = leaf.read().unwrap();
            Leaf::from_bytes(&lock).find_partition_lower(start)
        };
        ScanIterator::new(Arc::clone(&self.pool), leaf, end, start_idx)
    }

    /// Recursively inserts data into the page.
    ///
    /// The `page` argument may be a leaf node or a inner node
    fn insert_recurs(
        &self,
        page: PageRef<R>,
        object_id: ObjectID,
        tuple: &Tuple,
        mut parents: Vec<PageID>,
    ) {
        // Try an insert and store the result
        let insert_result = {
            let mut write_lock = page.write().unwrap();
            let page_repr = SlottedPage::from_bytes_mut(&mut write_lock);
            page_repr.insert(tuple)
        };

        // Check the result. If it is Ok, then we inserted!
        // If not, we need to get the parent and insert there.
        match insert_result {
            Ok(_) => {}
            Err(PageReadWriteError::OutOfSpace) => {
                // Do an insert and split, getting what we need to insert into parent
                let upstream_key = self.split_and_insert(&page, &tuple);

                // This parent will either be an existing node in the tree,
                // or a new parent node returned will have a left pointer to the original
                // page that we split.
                let parent_id = self.get_parent(&page, &mut parents, object_id);
                let parent_ref = self.pool.get_page_ref(parent_id).unwrap();

                // Now insert the sibling pointer into the parent that we got
                self.insert_recurs(parent_ref, object_id, &upstream_key, parents);
            }
            // Should not get anything else
            Err(e) => unreachable!("unexpected insert error: {e:?}"),
        }
    }

    fn get_parent(
        &self,
        page: &PageRef<R>,
        parents: &mut Vec<PageID>,
        object_id: ObjectID,
    ) -> PageID {
        match parents.pop() {
            Some(parent_id) => parent_id,
            None => {
                // Create new root. The left child ptr will be the current page id; sibling pointers are empty (NULL_PTR)
                let new_id = self.pool.new_page();
                let new_page = self.pool.get_page_ref(new_id).unwrap();
                self.pool.update_object_root(object_id, new_id);
                InnerNode::from_bytes_mut(&mut new_page.write().unwrap()).init(
                    new_id,
                    NULL_PTR,
                    NULL_PTR,
                    page.id(),
                );
                new_id
            }
        }
    }

    /// Splits the page and inserts the tuple, returning the tuple that needs to be inserted
    /// into the parent.
    fn split_and_insert(&self, page: &PageRef<R>, tuple: &Tuple) -> TupleBuf {
        // Split page, updating sibling pointers
        let new_sibling_id = self.pool.new_page();
        let new_sibling_page = self.pool.get_page_ref(new_sibling_id).unwrap();

        // Get locks
        let mut write_lock = page.write().unwrap();
        let mut new_write_lock = new_sibling_page.write().unwrap();

        // Check if we looking at a leaf or a node.
        // If a leaf, we need to just split and keep everything.
        // If a node, we need to take the middle key, then split
        match SlottedPage::from_bytes_mut(&mut write_lock)
            .get_header(&HeaderElem::PageType)
            .try_into()
            .unwrap()
        {
            PageType::Leaf => {
                let left_page_repr = Leaf::from_bytes_mut(&mut write_lock);
                let right_page_repr = Leaf::from_bytes_mut(&mut new_write_lock);

                let (middle_tuple, right_tuples) = left_page_repr.split_half(tuple);

                // Init sibling page
                right_page_repr.init(
                    new_sibling_id,
                    page.id(),
                    left_page_repr.get_header(&HeaderElem::RightSiblingPtr),
                );

                // Insert moved tuples into new page
                for moved_tuple in right_tuples.iter() {
                    right_page_repr.insert(&moved_tuple).unwrap();
                }

                // See if we insert original tuple that caused split left or right
                match tuple.key().bytes().cmp(middle_tuple.key().bytes()) {
                    std::cmp::Ordering::Less => left_page_repr.insert(tuple).unwrap(),
                    _ => right_page_repr.insert(tuple).unwrap(),
                }

                // Update original page to point right to new sibling page
                left_page_repr.set_header(
                    &HeaderElem::RightSiblingPtr,
                    right_page_repr.get_header(&HeaderElem::PageID),
                );

                TupleBuf::new(middle_tuple.key().bytes(), &new_sibling_id.to_be_bytes())
            }

            PageType::Node => {
                let left_page_repr = InnerNode::from_bytes_mut(&mut write_lock);
                let right_page_repr = InnerNode::from_bytes_mut(&mut new_write_lock);

                let (middle_tuple, right_tuples) = left_page_repr.split_half(tuple);

                // Init sibling page
                // The new left child ptr needs to be the page that the promoted
                // middle key used to point to
                right_page_repr.init(
                    new_sibling_id,
                    page.id(),
                    left_page_repr.get_header(&HeaderElem::RightSiblingPtr),
                    u32::from_be_bytes(middle_tuple.value().try_into().unwrap()),
                );

                for moved_tuple in right_tuples.iter() {
                    right_page_repr.insert(&moved_tuple).unwrap();
                }

                // Check if we insert original key left or right; or, if it is the middle key,
                // it gets promoted to the parent and does appear in the children
                match tuple.key().bytes().cmp(middle_tuple.key().bytes()) {
                    // Insert into left
                    std::cmp::Ordering::Less => left_page_repr.insert(tuple).unwrap(),
                    std::cmp::Ordering::Equal => {}
                    std::cmp::Ordering::Greater => right_page_repr.insert(tuple).unwrap(),
                }

                left_page_repr.set_header(
                    &HeaderElem::RightSiblingPtr,
                    right_page_repr.get_header(&HeaderElem::PageID),
                );

                middle_tuple
            }
        }
    }

    pub fn insert_tuple(&self, object_id: ObjectID, tuple: &Tuple) {
        // Get leaf page if not leaf page
        let root_id = self.pool.get_object_root(object_id);
        let (leaf, parents) = self.traverse_to_leaf(
            self.pool.get_page_ref(root_id).unwrap(),
            tuple.key().bytes(),
            Vec::new(),
        );

        // Call insert page which may become recursive if parents need to be split
        self.insert_recurs(leaf, object_id, tuple, parents);
    }

    /// Traverses the tree from `page` downward, following child pointers in inner
    /// nodes until a leaf is found whose range covers `key`.
    ///
    /// Returns the leaf [`PageRef`] together with the ordered stack of [`PageID`]s
    /// for every inner node visited along the way (nearest ancestor last), which
    /// callers use when propagating splits upward.
    ///
    /// # Potential improvements
    ///
    /// * **Replace recursion with a loop** — each recursive call only tail-calls
    ///   itself, so the entire function body can be rewritten as a `loop { ... }`
    ///   with no stack growth.
    /// * **Single lock per node** — the current implementation acquires the read
    ///   lock twice per inner node: once to read the page type and again to read
    ///   the child pointer. Both values can be extracted inside a single lock scope
    ///   to halve the locking overhead.
    fn traverse_to_leaf(
        &self,
        page: PageRef<R>,
        key: &[u8],
        mut parents: Vec<PageID>,
    ) -> (PageRef<R>, Vec<PageID>) {
        let page_type = {
            let lock = page.read().unwrap();
            PageType::new(SlottedPage::from_bytes(&lock).get_header(&HeaderElem::PageType)).unwrap()
        };
        match page_type {
            PageType::Leaf => {
                // If we found the leaf, just return the page and the current parent stack
                (page, parents)
            }
            PageType::Node => {
                // Get the child id
                let child_id = {
                    let lock = page.read().unwrap();
                    let repr = InnerNode::from_bytes(&lock);
                    repr.child(key)
                };

                // Push this onto the parents stack
                parents.push(page.id());

                // Recrusively call
                self.traverse_to_leaf(self.pool.get_page_ref(child_id).unwrap(), key, parents)
            }
        }
    }

    /// Delete a key from the B-Tree
    ///
    /// If this makes the page less than half full, then we need to check neighbors and maybe rearrange
    /// if they have any we could steal.
    ///
    /// As a first step, I think we can just always merge with neighbor.
    pub fn delete(&self) {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer_pool::BufferPool;
    use crate::representations::page::{Leaf, NULL_PTR};
    use crate::storage::MemoryIO;
    use proptest::prelude::*;
    use proptest::sample::SizeRange;
    use rand::seq::SliceRandom;

    fn create_test_tree() -> (BTree<MemoryIO>, ObjectID) {
        let pool = Arc::new(BufferPool::new(MemoryIO::default(), 1000));
        let (object_id, root) = pool.new_object_root();

        {
            let root_ref = pool.get_page_ref(root).unwrap();
            let mut root_lock = root_ref.write().unwrap();
            let root_repr = Leaf::from_bytes_mut(&mut root_lock);
            root_repr.init(root, NULL_PTR, NULL_PTR);
        }

        (BTree::new(pool), object_id)
    }

    fn unique_shuffled_vec(
        size: impl Into<proptest::collection::SizeRange>,
    ) -> impl Strategy<Value = Vec<u32>> {
        prop::collection::btree_set(any::<u32>(), size)
            .prop_map(|set| set.into_iter().collect::<Vec<_>>())
            .prop_shuffle()
    }

    // proptest! {
    //     #[test]
    //     fn test_tree_order(v in unique_shuffled_vec(1..10000)) {
    //         let (tree, object_id) = create_test_tree();

    //         for elem in v.iter() {
    //             let elem_bytes = elem.to_be_bytes();
    //             tree.insert_tuple(object_id, &TupleBuf::new(&elem_bytes, &elem_bytes));
    //         }

    //         let scanned: Vec<u32> = tree.iter_scan(object_id, &v.iter().min().unwrap().to_be_bytes(), &v.iter().max().unwrap().to_be_bytes()).map(|b| u32::from_be_bytes(b.try_into().unwrap())).collect();
    //         assert!(scanned.is_sorted())
    //     }
    // }

    #[test]
    fn test_tree_growth_and_scan() {
        let (tree, object_id) = create_test_tree();

        for i in 0..1u32 {
            let v = i.to_be_bytes();
            tree.insert_tuple(object_id, &TupleBuf::new(&v, &v));
            // println!("{}", i);
        }

        // for v in tree.iter_scan(root, &0u32.to_be_bytes(), &1000u32.to_be_bytes()) {
        //     println!("{}", u32::from_be_bytes(v.try_into().unwrap()))
        // }

        // for v in tree.iter_scan(root, &5u32.to_be_bytes(), &12u32.to_be_bytes()) {
        //     println!("{}", u32::from_be_bytes(v.try_into().unwrap()))
        // }

        for v in tree.iter_scan(object_id, &0u32.to_be_bytes(), &500u32.to_be_bytes()) {
            println!("{}", u32::from_be_bytes(v.try_into().unwrap()))
        }
    }
}
