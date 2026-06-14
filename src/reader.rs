use crate::reader::HeaderElem::FreeSpace;

use super::buffer_pool::{PAGE_SIZE, PageID};
use std::{io::Read, ops::Deref, slice::SliceIndex};

/// Page type in the BTree. Node pages map keys to page ids, leaf pages map keys to tuples
#[derive(Debug, PartialEq)]
pub enum PageType {
    /// Nodes always map to Key => PageID
    Node,
    /// Leaf always maps Key => Tuple
    Leaf,
}

impl PageType {
    pub fn id(&self) -> u64 {
        match self {
            Self::Node => 0,
            Self::Leaf => 1,
        }
    }

    pub fn new(type_id: u64) -> Self {
        match type_id {
            0 => Self::Node,
            1 => Self::Leaf,
            _ => panic!(),
        }
    }
}

pub enum HeaderElem {
    PageID,
    PageType,
    FreeSpace,
    LastCommit,
    ItemCount,
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
            Self::LeftPtr => 24,
            Self::RightPtr => 28,
            Self::FreeSpacePtr => 32,
        }
    }
}

const HEADER_SIZE: usize = 4 * 8;

pub trait PageReader {
    fn get_header(&self, element: HeaderElem) -> u32;
    /// Returns K, V pair
    fn tuple(&self, idx: usize) -> Option<(&[u8], &[u8])>;
    fn tuple_unsafe(&self, idx: usize) -> (&[u8], &[u8]);
    fn find_key(&self, key: &[u8], low: usize, high: usize) -> Option<usize>;
    fn get_slot_ptr(&self, slot: usize) -> Option<usize>;
    fn get_slot_ptr_unsafe(&self, slot: usize) -> usize;
    fn content(&self) -> &[u8];
    fn find_child(&self, key: &[u8]) -> PageID;
}

pub trait PageWriter {
    fn set_header(&mut self, element: HeaderElem, value: u32);
    fn init(&mut self, page_id: PageID, page_type: PageType, left_ptr: PageID, right_ptr: PageID);
    fn insert(&mut self, data: &[u8]) -> Result<(), PageWriteError>;
    fn content_mut(&mut self) -> &mut [u8];
}

impl PageReader for [u8] {
    fn get_header(&self, element: HeaderElem) -> u32 {
        let offset = element.offset();
        u32::from_be_bytes(self[offset..offset + 4].try_into().unwrap())
    }

    /// tuple has layout \[tuple size (u16)] \[key size (u16) | key ()] \[value]
    fn tuple(&self, idx: usize) -> Option<(&[u8], &[u8])> {
        // Read first u16 / get size
        let ptr = self.get_slot_ptr(idx)?;
        let tuple_size = u16::from_be_bytes(self.content()[ptr..ptr + 2].try_into().unwrap());
        let key_size = u16::from_be_bytes(self.content()[ptr + 2..ptr + 4].try_into().unwrap());

        // Split at key end
        let ks = ptr + 4;
        Some(self.content()[ks..ks + tuple_size as usize].split_at(ks + key_size as usize))
    }

    fn tuple_unsafe(&self, idx: usize) -> (&[u8], &[u8]) {
        // Read first u16 / get size
        let ptr = self.get_slot_ptr_unsafe(idx.into());
        let tuple_size = u16::from_be_bytes(self.content()[ptr..ptr + 2].try_into().unwrap());
        let key_size = u16::from_be_bytes(self.content()[ptr + 2..ptr + 4].try_into().unwrap());

        // Split at key end
        let ks = ptr + 4;
        self.content()[ks..ks + tuple_size as usize].split_at(ks + key_size as usize)
    }

    fn find_key(&self, key: &[u8], low: usize, high: usize) -> Option<usize> {
        // Initize index to middle of slot array
        let idx = (high - low) / 2 + low;
        let tkey = self.tuple_unsafe(idx).0;

        if tkey == key {
            return Some(idx);
        }

        if low == high {
            return None;
        }

        if tkey < key {
            self.find_key(key, idx + 1, high)
        } else {
            if idx == 0 {
                return None;
            } else {
                self.find_key(key, low, idx - 1)
            }
        }
    }

    /// Just iterate through page
    fn find_child(&self, key: &[u8]) -> PageID {
        let mut ptr = 0usize;
        let mut idx = 0;
        let items = self.get_header(HeaderElem::ItemCount);
        loop {
            // read page id
            let page_id = u64::from_be_bytes(self[ptr..ptr + 8].try_into().unwrap());
            idx += 1;

            // end of the line
            if idx >= items {
                break page_id;
            }
            ptr += 8;

            // read key length
            let length = u16::from_be_bytes(self[ptr..ptr + 2].try_into().unwrap()) as usize;
            ptr += 2;

            // read and compare key. If next key is greater, then break with last page id
            if self[ptr..ptr + length] > *key {
                break page_id;
            }
            ptr += length;
        }
    }

    /// Caller must verify that the slot is in bounds
    fn get_slot_ptr_unsafe(&self, slot: usize) -> usize {
        u16::from_be_bytes(
            self.content()[slot * 2..(slot + 1) * 2]
                .try_into()
                .unwrap(),
        ) as usize
    }

    fn get_slot_ptr(&self, slot: usize) -> Option<usize> {
        if self.get_header(HeaderElem::ItemCount) >= slot as u32 {
            None
        } else {
            Some(u16::from_be_bytes(
                self.content()[slot * 2..(slot + 1) * 2]
                    .try_into()
                    .unwrap(),
            ) as usize)
        }
    }

    fn content(&self) -> &[u8] {
        &self[HEADER_SIZE..self.len()]
    }
}

#[derive(Debug, thiserror::Error)]
enum PageWriteError {
    #[error("Page is out of space for insert")]
    OutOfSpace,
}

impl PageWriter for [u8] {
    fn content_mut(&mut self) -> &mut [u8] {
        let len = self.len();
        &mut self[HEADER_SIZE..len]
    }

    fn init(&mut self, page_id: PageID, page_type: PageType, left_ptr: PageID, right_ptr: PageID) {
        self.set_header(HeaderElem::PageID, page_id.try_into().unwrap());
        self.set_header(HeaderElem::PageType, page_type.id().try_into().unwrap());
        self.set_header(
            HeaderElem::FreeSpace,
            (PAGE_SIZE - HEADER_SIZE).try_into().unwrap(),
        ); // 2 bytes for first slot
        self.set_header(HeaderElem::ItemCount, 0);
        self.set_header(HeaderElem::LeftPtr, left_ptr.try_into().unwrap());
        self.set_header(HeaderElem::RightPtr, right_ptr.try_into().unwrap());
        self.set_header(HeaderElem::FreeSpacePtr, PAGE_SIZE.try_into().unwrap());
    }

    fn insert(&mut self, data: &[u8]) -> Result<(), PageWriteError> {
        // First, check free space
        let free_space = self.get_header(HeaderElem::FreeSpace);
        if (data.len() + 2usize) < free_space as usize {
            return Err(PageWriteError::OutOfSpace);
        }

        // Get slot write ptr and tuple write ptr
        let item_count = self.get_header(HeaderElem::ItemCount);
        let slot_write_ptr = item_count as usize * 2usize;
        let tuple_write_ptr = self.get_header(HeaderElem::FreeSpacePtr) as usize - data.len();

        // I think the safest order is probably write out Tuple, then Slot, then update headers?
        // Write first
        let mut_content = self.content_mut();
        mut_content[tuple_write_ptr..tuple_write_ptr + data.len()].copy_from_slice(data);
        mut_content[slot_write_ptr..slot_write_ptr + 2]
            .copy_from_slice(&(tuple_write_ptr as u16).to_be_bytes());

        // Update header
        self.set_header(HeaderElem::FreeSpace, free_space - data.len() as u32);
        self.set_header(HeaderElem::FreeSpacePtr, tuple_write_ptr as u32);
        self.set_header(HeaderElem::ItemCount, item_count + 1);

        Ok(())
    }

    fn set_header(&mut self, element: HeaderElem, value: u32) {}
}
