use std::{borrow::Borrow, ops::Deref};

/// DST represnting a key in data.
///
/// Key has 2 bytes for the size in u16, then the serialized bytes
#[repr(transparent)]
#[derive(Debug, PartialEq)]
pub struct Key([u8]);

impl Key {
    pub fn from_bytes(bytes: &[u8]) -> &Self {
        unsafe { &*(bytes as *const [u8] as *const Self) }
    }

    pub fn size(&self) -> u16 {
        u16::from_be_bytes(self.0[0..2].try_into().unwrap())
    }

    pub fn bytes(&self) -> &[u8] {
        &self.0[2..self.0.len()]
    }
}

impl ToOwned for Key {
    type Owned = KeyBuf;

    fn to_owned(&self) -> Self::Owned {
        KeyBuf::new(&self.0)
    }
}

#[derive(Debug, PartialEq)]
pub struct KeyBuf {
    bytes: Vec<u8>,
}

impl KeyBuf {
    pub fn new(key: &[u8]) -> Self {
        let size = key.len() + 2; // allocate size of key + 2 more for this size
        let mut data = Vec::<u8>::with_capacity(size);
        data.extend_from_slice((size as u16).to_be_bytes().as_slice());
        data.extend_from_slice(key);
        KeyBuf { bytes: data }
    }
}

impl Deref for KeyBuf {
    type Target = Key;

    fn deref(&self) -> &Self::Target {
        Key::from_bytes(&self.bytes)
    }
}

impl std::borrow::Borrow<Key> for KeyBuf {

    fn borrow(&self) -> &Key {
        Key::from_bytes(&self.bytes)
    }
}

/// DST representing a tuple in data.
///
/// This has the following format:
///
/// \[Header(size: u16) | Key(size: u16, *bytes) | Value(*bytes) ]
#[repr(transparent)]
#[derive(Debug, PartialEq)]
pub struct Tuple(pub [u8]);

impl Tuple {
    pub fn from_bytes(bytes: &[u8]) -> &Self {
        unsafe { &*(bytes as *const [u8] as *const Tuple) }
    }

    pub fn size(&self) -> u16 {
        u16::from_be_bytes(self.0[0..2].try_into().unwrap())
    }

    pub fn key_size(&self) -> u16 {
        u16::from_be_bytes(self.0[2..4].try_into().unwrap())
    }

    pub fn key(&self) -> &Key {
        let key_size = self.key_size();
        Key::from_bytes(&self.0[2..2 + 2 + key_size as usize])
    }

    pub fn value(&self) -> &[u8] {
        let key_size = self.key_size() as usize;
        &self.0[4 + key_size..self.0.len()]
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl std::fmt::Display for Tuple {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[ {:x} | [ {:x} | {:?} ] | [ {:x?} ] ]",
            self.size(),
            self.key_size(),
            self.key(),
            self.value()
        )
    }
}

impl ToOwned for Tuple {
    type Owned = TupleBuf;

    fn to_owned(&self) -> Self::Owned {
        TupleBuf {bytes: self.0.to_vec()}
    }
}

#[derive(Debug, PartialEq)]
pub struct TupleBuf {
    bytes: Vec<u8>,
}

impl TupleBuf {
    pub fn new(key: &[u8], value: &[u8]) -> Self {
        // add key size + value size + total tuple size (tuple + key headers)
        let size = key.len() + value.len() + 4;
        let mut data = Vec::<u8>::with_capacity(size);

        // extend with data
        data.extend_from_slice((size as u16).to_be_bytes().as_slice());
        data.extend_from_slice((key.len() as u16).to_be_bytes().as_slice());
        data.extend_from_slice(key);
        data.extend_from_slice(value);
        TupleBuf { bytes: data }
    }
}

impl Deref for TupleBuf {
    type Target = Tuple;

    fn deref(&self) -> &Self::Target {
        Tuple::from_bytes(&self.bytes)
    }
}

impl std::borrow::Borrow<Tuple> for TupleBuf {
    
    fn borrow(&self) -> &Tuple {
        Tuple::from_bytes(&self.bytes)
    }
}

impl std::fmt::Display for TupleBuf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.deref().fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

}
