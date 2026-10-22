//! The game's executable (main.dol): read values at their virtual addresses, for
//! tables and strings the game keeps in code rather than in data files.

/// A DOL image: 7 text and 11 data sections, big-endian.
pub struct Dol {
    data: Vec<u8>,
}

impl Dol {
    pub fn new(data: Vec<u8>) -> Dol {
        Dol { data }
    }

    fn be(&self, o: usize) -> Option<u32> {
        Some(u32::from_be_bytes(
            self.data.get(o..o + 4)?.try_into().ok()?,
        ))
    }

    /// File offset of a virtual address, if a section holds it.
    pub fn offset(&self, addr: u32) -> Option<usize> {
        (0..18).find_map(|i| {
            let (off, a, sz) = (
                self.be(i * 4)?,
                self.be(0x48 + i * 4)?,
                self.be(0x90 + i * 4)?,
            );
            (addr >= a && addr < a.wrapping_add(sz)).then(|| (off + addr - a) as usize)
        })
    }

    pub fn u32_at(&self, addr: u32) -> Option<u32> {
        self.be(self.offset(addr)?)
    }

    pub fn f32_at(&self, addr: u32) -> Option<f32> {
        self.u32_at(addr).map(f32::from_bits)
    }

    /// The NUL-terminated string at an address.
    pub fn str_at(&self, addr: u32) -> Option<String> {
        let o = self.offset(addr)?;
        let s = self.data.get(o..)?;
        let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
        Some(String::from_utf8_lossy(&s[..end]).into_owned())
    }

    /// The string a `char*` table entry points at.
    pub fn str_ptr(&self, table: u32, index: u32) -> Option<String> {
        self.str_at(self.u32_at(table + 4 * index)?)
    }
}
