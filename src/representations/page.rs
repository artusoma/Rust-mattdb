use std::ops::{Deref, DerefMut};

use super::tuple::*;
use crate::buffer_pool::PAGE_SIZE;

pub const NULL_PTR: u32 = u32::MAX;

#[derive(Debug, thiserror::Error)]
pub enum PageReadWriteError {
    #[error("Page is out of space for insert")]
    OutOfSpace,
    #[error("Cannot find specified key (bytes: {0})")]
    KeyNotFound(String),
    #[error("Page type error: cannot interpret page type `{0}`")]
    PageTypeError(u32),
}

pub enum HeaderElem {
    PageID,
    PageType,
    ContFreeSpace,
    LastCommit,
    ItemCount,
    LeftSiblingPtr,
    RightSiblingPtr,
    FreeSpacePtr,
    LeftChildPtr,
    TotalFreeSpace,
}

impl HeaderElem {
    fn offset(&self) -> usize {
        match self {
            Self::PageID => 0,
            Self::PageType => 4,
            Self::ContFreeSpace => 8,
            Self::LastCommit => 12,
            Self::ItemCount => 16,
            Self::LeftSiblingPtr => 20,
            Self::RightSiblingPtr => 24,
            Self::FreeSpacePtr => 28,
            Self::LeftChildPtr => 32,
            Self::TotalFreeSpace => 36,
        }
    }
}

const HEADER_SIZE: usize = 4 * 10;

/// Page type in the BTree. Node pages map keys to page ids, leaf pages map keys to tuples
#[derive(Debug, PartialEq)]
pub enum PageType {
    /// Nodes always map to Key => PageID
    Node,
    /// Leaf always maps Key => Tuple
    Leaf,
}

impl PageType {
    pub fn id(&self) -> u32 {
        match self {
            Self::Node => 0,
            Self::Leaf => 1,
        }
    }

    pub fn new(type_id: u32) -> Result<Self, PageReadWriteError> {
        match type_id {
            0 => Ok(Self::Node),
            1 => Ok(Self::Leaf),
            _ => Err(PageReadWriteError::PageTypeError(type_id)),
        }
    }
}

impl TryFrom<u32> for PageType {
    type Error = PageReadWriteError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        PageType::new(value)
    }
}

/// Represents space available on an insert
enum SpaceStatus {
    Ok,
    NeedsCollapse,
    OutOfSpace,
}

/// Single shared representation for a slotted page and its ensuing operations
#[repr(transparent)]
pub struct SlottedPage([u8]);

impl SlottedPage {
    /*
    INIT OPERATIONS
    */
    pub fn from_bytes(bytes: &[u8]) -> &Self {
        unsafe { &*(bytes as *const [u8] as *const Self) }
    }

    pub fn from_bytes_mut(bytes: &mut [u8]) -> &mut Self {
        unsafe { &mut *(bytes as *mut [u8] as *mut Self) }
    }

    /*
    HEADER OPERATIONS
    */

    pub fn get_header(&self, element: &HeaderElem) -> u32 {
        let offset = element.offset();
        u32::from_be_bytes(self.0[offset..offset + 4].try_into().unwrap())
    }

    pub fn set_header(&mut self, element: &HeaderElem, value: u32) {
        let offset = element.offset();
        self.0[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    }

    pub fn update_header(&mut self, element: &HeaderElem, diff: i64) {
        let current = self.get_header(element);
        self.set_header(element, ((current as i64) + diff) as u32);
    }

    pub fn percent_full(&self) -> u8 {
        100 - (self.get_header(&HeaderElem::ContFreeSpace) as usize * 100
            / (PAGE_SIZE - HEADER_SIZE)) as u8
    }

    /*
    B-TREE OPERATIONS
    */

    pub fn init(
        &mut self,
        page_id: u32,
        page_type: PageType,
        left_ptr: u32,
        right_ptr: u32,
        left_child_ptr: u32,
    ) {
        self.set_header(&HeaderElem::PageID, page_id.try_into().unwrap());
        self.set_header(&HeaderElem::PageType, page_type.id().try_into().unwrap());
        self.set_header(
            &HeaderElem::ContFreeSpace,
            (PAGE_SIZE - HEADER_SIZE).try_into().unwrap(),
        ); // 2 bytes for first slot
        self.set_header(
            &HeaderElem::TotalFreeSpace,
            (PAGE_SIZE - HEADER_SIZE).try_into().unwrap(),
        ); // 2 bytes for first slot
        self.set_header(&HeaderElem::ItemCount, 0);
        self.set_header(&HeaderElem::LeftSiblingPtr, left_ptr.try_into().unwrap());
        self.set_header(&HeaderElem::RightSiblingPtr, right_ptr.try_into().unwrap());
        self.set_header(&HeaderElem::FreeSpacePtr, PAGE_SIZE.try_into().unwrap());

        self.set_header(
            &HeaderElem::LeftChildPtr,
            left_child_ptr.try_into().unwrap(),
        )
    }

    pub fn tuple(&self, idx: usize) -> Option<&Tuple> {
        // Read first u16 / get size
        let ptr = if self.get_header(&HeaderElem::ItemCount) <= idx as u32 {
            None
        } else {
            let slot_ptr = HEADER_SIZE + idx * 2;
            Some(u16::from_be_bytes(self.0[slot_ptr..slot_ptr + 2].try_into().unwrap()) as usize)
        }?;
        let tuple_size = u16::from_be_bytes(self.0[ptr..ptr + 2].try_into().unwrap()) as usize;
        Some(Tuple::from_bytes(&self.0[ptr..ptr + tuple_size]))
    }

    pub fn find_key(&self, key: &[u8]) -> Option<usize> {
        let count = self.get_header(&HeaderElem::ItemCount);
        if count == 0 {
            None
        } else {
            self.find_key_inner(key, 0, self.get_header(&HeaderElem::ItemCount) as usize - 1)
        }
    }

    fn find_key_inner(&self, key: &[u8], mut low: usize, mut high: usize) -> Option<usize> {
        loop {
            let idx = low + (high - low) / 2;

            let this_key = self.tuple(idx).unwrap().key().bytes();

            if high == low {
                return if this_key == key { Some(high) } else { None };
            }

            match this_key.cmp(key) {
                std::cmp::Ordering::Equal => return Some(idx),
                std::cmp::Ordering::Less => low = idx + 1,
                std::cmp::Ordering::Greater => {
                    if idx == low {
                        high = high - 1;
                    } else {
                        high = idx - 1;
                    }
                }
            }
        }
    }

    pub fn find_partition(&self, key: &[u8]) -> usize {
        let count = self.get_header(&HeaderElem::ItemCount);
        if count == 0 {
            0
        } else {
            self.find_partition_inner(key, 0, count as usize - 1)
        }
    }

    /// Returns the index at which `key` should be inserted to keep the slot array sorted,
    /// searching within the inclusive index range `[low, high]`.
    ///
    /// For a page containing keys `[3, 5, 5, 7]`:
    /// - `key = 2` → `0` (insert before everything)
    /// - `key = 5` → `3` (insert after the last `5`, preserving append order for duplicates)
    /// - `key = 9` → `4` (insert after everything)
    ///
    /// The implementation is an iterative binary search. Each branch narrows the window:
    /// - `Less`: raise `low` past the midpoint — `key` belongs to the right.
    /// - `Equal`: lower `high` to the midpoint — scan left to find the first match,
    ///   so the returned index lands after all equal keys.
    /// - `Greater`: lower `high` below the midpoint — `key` belongs to the left.
    ///   Returns `0` immediately when `idx == 0` to avoid underflow.
    fn find_partition_inner(&self, key: &[u8], mut low: usize, mut high: usize) -> usize {
        loop {
            let idx = low + (high - low) / 2;

            let this_key = self.tuple(idx).unwrap().key().bytes();
            if high == low {
                return match this_key.cmp(key) {
                    std::cmp::Ordering::Less | std::cmp::Ordering::Equal => idx + 1,
                    _ => idx,
                };
            }
            match this_key.cmp(key) {
                std::cmp::Ordering::Equal => {
                    high = idx;
                }
                std::cmp::Ordering::Less => low = idx + 1,
                std::cmp::Ordering::Greater => {
                    if idx == low {
                        high = high - 1
                    } else {
                        high = idx - 1;
                    }
                }
            }
        }
    }

    /// Determines whether the page has sufficient space for an insertion of `required` bytes.
    ///
    /// Distinguishes between two kinds of free space:
    ///
    /// - [`HeaderElem::ContFreeSpace`]: contiguous free space at the end of the data region.
    ///   An insert can proceed immediately if this is large enough.
    /// - [`HeaderElem::TotalFreeSpace`]: total free bytes including fragmented gaps left by
    ///   previous deletions. If only this is sufficient, the page must be
    ///   [collapsed](Self::collapse) first to compact the fragments into contiguous space.
    ///
    /// # Returns
    ///
    /// - [`SpaceStatus::Ok`] — enough contiguous space; insert can proceed directly.
    /// - [`SpaceStatus::NeedsCollapse`] — enough total space but fragmented; compact first.
    /// - [`SpaceStatus::OutOfSpace`] — not enough space even after compaction; page must be split.
    fn check_space(&self, required: usize) -> SpaceStatus {
        if self.get_header(&HeaderElem::ContFreeSpace) as usize >= required {
            SpaceStatus::Ok
        } else if self.get_header(&HeaderElem::TotalFreeSpace) as usize >= required {
            SpaceStatus::NeedsCollapse
        } else {
            SpaceStatus::OutOfSpace
        }
    }

    pub fn insert(&mut self, data: &Tuple) -> Result<(), PageReadWriteError> {
        // We need to check how much space we have.
        match self.check_space(data.len() + 2usize) {
            SpaceStatus::Ok => {}
            SpaceStatus::NeedsCollapse => self.collapse(),
            SpaceStatus::OutOfSpace => return Err(PageReadWriteError::OutOfSpace),
        }

        // Get slot write ptr and tuple write ptr
        let tuple_write_ptr = self.get_header(&HeaderElem::FreeSpacePtr) as usize - data.len();
        let slot_write_idx = self.find_partition(data.key().bytes());

        // I think the safest order is probably write out Tuple, then Slot, then update headers?
        // Write first
        // let mut_content = self.content_mut();
        self.0[tuple_write_ptr..tuple_write_ptr + data.len()].copy_from_slice(&data.0);
        self.insert_slot_at(slot_write_idx, tuple_write_ptr.try_into().unwrap());

        // Update header
        self.update_header_insert(data.size().into());

        Ok(())
    }

    fn update_header_insert(&mut self, insert_size: i64) {
        self.update_header(&HeaderElem::FreeSpacePtr, -insert_size);
        self.update_header(&HeaderElem::ItemCount, 1);
        self.update_header(&HeaderElem::ContFreeSpace, -insert_size - 2);
        self.update_header(&HeaderElem::TotalFreeSpace, -insert_size - 2);
    }

    /// Inserts a slot pointer at position `idx` in the slot array, shifting all
    /// slots at `idx..item_count` one position to the right to make room.
    ///
    /// The slot array grows toward higher byte offsets (away from the header),
    /// while tuple data grows downward from [`HeaderElem::FreeSpacePtr`]. It is
    /// the caller's responsibility to ensure that inserting this slot — which
    /// consumes 2 bytes — will not cause the slot array to collide with the
    /// tuple data region. [`Self::check_space`] enforces this precondition
    /// before calling this function.
    fn insert_slot_at(&mut self, idx: usize, ptr: u16) {
        // Get pointer to where the slot should go
        let item_count = self.get_header(&HeaderElem::ItemCount) as usize;
        let start_ptr = HEADER_SIZE + idx * 2;

        // check if we have items to move over
        if idx < item_count {
            let end_ptr = HEADER_SIZE + (item_count - 1) * 2;
            self.0.copy_within(start_ptr..end_ptr + 2, start_ptr + 2);
        }

        // Write new slot
        self.0[start_ptr..start_ptr + 2].copy_from_slice(&ptr.to_be_bytes());
    }

    /// Removes a key-value pair from the page.
    ///
    /// Locates the tuple with the given key and removes it, shifting the remaining
    /// slot pointers to fill the gap. The tuple data itself remains in memory but becomes
    /// unreachable, contributing to fragmentation that can be recovered via [`collapse`](Self::collapse).
    ///
    /// # Arguments
    ///
    /// * `key` — the key bytes to search for and delete.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` on successful deletion, or `KeyNotFound` if the key does not exist.
    ///
    /// # Behavior
    ///
    /// - Updates [`ItemCount`](HeaderElem::ItemCount) to reflect the new number of items.
    /// - Updates [`TotalFreeSpace`](HeaderElem::TotalFreeSpace) to account for the freed tuple data
    ///   and its 2-byte slot.
    /// - Leaves the tuple data and any gaps in memory (fragmentation). Use [`collapse`](Self::collapse)
    ///   to reclaim fragmented space.
    pub fn delete(&mut self, key: &[u8]) -> Result<(), PageReadWriteError> {
        let delete_idx = self
            .find_key(key)
            .ok_or(PageReadWriteError::KeyNotFound(format!("{:?}", key)))?;

        // Get size of tuple to mark the free space
        let tuple_size = self.tuple(delete_idx).unwrap().size();

        // Move everything over to cover now deleted key.
        // The `end_ptr` maps from current item count to copy end.
        // Ex: if 1 item, then the end_ptr will be 2.
        let current_count = self.get_header(&HeaderElem::ItemCount);
        if current_count > 1 {
            let start_ptr = HEADER_SIZE + delete_idx * 2 + 2;
            let end_ptr = HEADER_SIZE + current_count as usize * 2;
            self.0.copy_within(start_ptr..end_ptr, start_ptr - 2);
        }

        // Set new item count and free space
        self.update_header(&HeaderElem::ItemCount, -1);
        self.update_header(&HeaderElem::TotalFreeSpace, -(tuple_size as i64 + 2));

        Ok(())
    }

    /// Cleans up the page, only keeping the first 0..split_idx items in the page
    fn keep_left(&mut self, split_idx: usize) {
        // Create new temporary page to copy left into, then replace self.0
        // Use a box to avoid allocating 8kb on the stack...
        let mut temp_bytes = Box::new([0u8; PAGE_SIZE]);
        let temp = SlottedPage::from_bytes_mut(&mut *temp_bytes);
        temp.init(
            self.get_header(&HeaderElem::PageID),
            self.get_header(&HeaderElem::PageType).try_into().unwrap(), // unwrap; cannot fail
            self.get_header(&HeaderElem::LeftSiblingPtr),
            self.get_header(&HeaderElem::RightSiblingPtr),
            self.get_header(&HeaderElem::LeftChildPtr),
        );
        for idx in 0..split_idx {
            temp.insert(self.tuple(idx).unwrap()).unwrap();
        }
        self.0.copy_from_slice(&temp.0[0..PAGE_SIZE]);
    }

    /// Collapse empty space in a page, so that [`HeaderElem::ContFreeSpace`]
    /// and [`HeaderElem::TotalFreeSpace`] match.
    pub fn collapse(&mut self) {
        self.keep_left(self.get_header(&HeaderElem::ItemCount) as usize);
    }

    pub fn split_half(&mut self, tuple: &Tuple) -> (TupleBuf, Vec<TupleBuf>) {
        // Get how many items vs keep vs remove. Left is kept, right is moved.
        let item_count = self.get_header(&HeaderElem::ItemCount) as usize + 1;

        // Get idx where this key would go
        let key_loc = self.find_partition(&tuple.key().bytes());
        let middle_idx = item_count / 2;

        let (split_idx, middle_tuple) = match key_loc.cmp(&middle_idx) {
            std::cmp::Ordering::Less => (
                middle_idx - 1,
                self.tuple(middle_idx - 1).unwrap().to_owned(),
            ),
            std::cmp::Ordering::Equal => (middle_idx, tuple.to_owned()),
            std::cmp::Ordering::Greater => (middle_idx, self.tuple(middle_idx).unwrap().to_owned()),
        };

        // Grab tuples that will go right
        let mut tuples: Vec<TupleBuf> = Vec::with_capacity(item_count - split_idx - 1);
        for idx in split_idx..(item_count - 1) {
            tuples.push(self.tuple(idx).unwrap().to_owned());
        }

        // Only keep left side of the page
        self.keep_left(split_idx);

        (middle_tuple, tuples)
    }
}

/// DST representing a leaf node.
///
/// Leaf node derefs right through to the slotted page.
#[repr(transparent)]
pub struct Leaf(SlottedPage);

impl Leaf {
    pub fn from_bytes(bytes: &[u8]) -> &Self {
        unsafe { &*(bytes as *const [u8] as *const SlottedPage as *const Leaf) }
    }

    pub fn from_bytes_mut(bytes: &mut [u8]) -> &mut Self {
        unsafe { &mut *(bytes as *mut [u8] as *mut SlottedPage as *mut Leaf) }
    }

    pub fn init(&mut self, page_id: u32, left_ptr: u32, right_ptr: u32) {
        self.0
            .init(page_id, PageType::Leaf, left_ptr, right_ptr, NULL_PTR);
    }
}

impl Deref for Leaf {
    type Target = SlottedPage;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Leaf {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// DST represetnign a inner node.
///
/// This is a special case of the leaf / slotted page,
/// and so we wrap methods.
#[repr(transparent)]
pub struct InnerNode(SlottedPage);

impl InnerNode {
    pub fn from_bytes(bytes: &[u8]) -> &Self {
        unsafe { &*(bytes as *const [u8] as *const SlottedPage as *const InnerNode) }
    }

    pub fn from_bytes_mut(bytes: &mut [u8]) -> &mut Self {
        unsafe { &mut *(bytes as *mut [u8] as *mut SlottedPage as *mut InnerNode) }
    }

    pub fn init(&mut self, page_id: u32, left_ptr: u32, right_ptr: u32, left_child_ptr: u32) {
        self.0
            .init(page_id, PageType::Node, left_ptr, right_ptr, left_child_ptr);
    }

    /// Get the next child page in the search for `key`
    ///
    /// If we use base find partition, then we get an index back.
    /// We actually need to shift everything back one.
    ///
    /// Say we have twos keys (4, 7).
    /// - If `key = 2` => `left_child_ptr`  , but `find_partition` returns `0`
    /// - If `key = 4` => value at `idx = 0`, but `find_partition` returns `1`
    /// - If `key = 5` => value at `idx = 0`, but `find_partition` returns `1`
    /// - If `key = 7` => value at `idx = 1`, but `find_partition` returns `2`
    pub fn child(&self, key: &[u8]) -> u32 {
        let found_idx = self.0.find_partition(key);
        if found_idx == 0 {
            self.0.get_header(&HeaderElem::LeftChildPtr)
        } else {
            u32::from_be_bytes(
                self.0
                    .tuple(found_idx - 1)
                    .unwrap()
                    .value()
                    .try_into()
                    .unwrap(),
            )
        }
    }
}

impl Deref for InnerNode {
    type Target = SlottedPage;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for InnerNode {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_page_split() {
        let mut bytes = vec![0u8; PAGE_SIZE];
        let page = Leaf::from_bytes_mut(&mut bytes);
        page.init(1, NULL_PTR, NULL_PTR);

        for i in 0..50u32 {
            let bytes = i.to_be_bytes();
            let t = TupleBuf::new(&bytes, &bytes);
            page.insert(&t).unwrap();
        }

        for i in 51..100u32 {
            let bytes = i.to_be_bytes();
            let t = TupleBuf::new(&bytes, &bytes);
            page.insert(&t).unwrap();
        }

        let bytes = 50u32.to_be_bytes();
        let t = TupleBuf::new(&bytes, &bytes);
        let (middle, right) = page.split_half(&t);

        assert_eq!(t, middle);

        let bytes = 51u32.to_be_bytes();
        let t = TupleBuf::new(&bytes, &bytes);
        assert_eq!(*right.get(0).unwrap(), t);
    }

    #[test]
    fn test_page_insert_delete() {
        use rand::seq::SliceRandom;

        let mut bytes = vec![0u8; PAGE_SIZE];
        let page = Leaf::from_bytes_mut(&mut bytes);
        page.init(1, NULL_PTR, NULL_PTR);

        // insert 100 random tuples where key = value
        let mut idxs: Vec<u32> = (0..100u32).collect();
        idxs.shuffle(&mut rand::rng());
        for i in &idxs {
            let bytes = i.to_be_bytes();
            let t = TupleBuf::new(&bytes, &bytes);
            page.insert(&t).unwrap();
        }

        // check that we are in order
        for i in 0..100usize {
            let t = page.tuple(i).unwrap();
            let k = u32::from_be_bytes(t.key().bytes().try_into().unwrap());
            assert_eq!(i, k as usize)
        }

        // delete in arbitrary order
        for i in &idxs {
            let bytes = i.to_be_bytes();
            page.delete(&bytes).unwrap();
        }

        // cleanup and check we have restored space
        page.collapse();
        assert_eq!(
            PAGE_SIZE - HEADER_SIZE,
            page.get_header(&HeaderElem::ContFreeSpace) as usize
        )
    }
}
