//! Little helpers for reading fixed-layout binary data.

use anyhow::{bail, Result};

/// Bounds-checked little-endian reader over a byte slice.
#[derive(Clone, Copy)]
pub struct Le<'a> {
    pub data: &'a [u8],
}

impl<'a> Le<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Le { data }
    }

    fn get<const N: usize>(&self, at: usize) -> Result<[u8; N]> {
        match self.data.get(at..at + N) {
            Some(b) => Ok(b.try_into().unwrap()),
            None => bail!(
                "read of {N} bytes at {at:#x} past end ({:#x})",
                self.data.len()
            ),
        }
    }

    pub fn u8(&self, at: usize) -> Result<u8> {
        Ok(self.get::<1>(at)?[0])
    }
    pub fn i8(&self, at: usize) -> Result<i8> {
        Ok(self.get::<1>(at)?[0] as i8)
    }
    pub fn u16(&self, at: usize) -> Result<u16> {
        Ok(u16::from_le_bytes(self.get(at)?))
    }
    pub fn i16(&self, at: usize) -> Result<i16> {
        Ok(i16::from_le_bytes(self.get(at)?))
    }
    pub fn u32(&self, at: usize) -> Result<u32> {
        Ok(u32::from_le_bytes(self.get(at)?))
    }
    pub fn i32(&self, at: usize) -> Result<i32> {
        Ok(i32::from_le_bytes(self.get(at)?))
    }
    pub fn f32(&self, at: usize) -> Result<f32> {
        Ok(f32::from_le_bytes(self.get(at)?))
    }
    pub fn vec3(&self, at: usize) -> Result<[f32; 3]> {
        Ok([self.f32(at)?, self.f32(at + 4)?, self.f32(at + 8)?])
    }
    /// NUL-terminated string stored in a fixed-size field.
    pub fn str(&self, at: usize, len: usize) -> Result<String> {
        let Some(b) = self.data.get(at..at + len) else {
            bail!("string of {len} bytes at {at:#x} past end");
        };
        let end = b.iter().position(|&c| c == 0).unwrap_or(len);
        Ok(String::from_utf8_lossy(&b[..end]).into_owned())
    }
    pub fn bytes(&self, at: usize, len: usize) -> Result<&'a [u8]> {
        match self.data.get(at..at + len) {
            Some(b) => Ok(b),
            None => bail!(
                "slice of {len} bytes at {at:#x} past end ({:#x})",
                self.data.len()
            ),
        }
    }
}
