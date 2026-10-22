//! Minimal GameCube disc reader.
//!
//! Supports plain GCM/ISO images and NKit-scrubbed `.nkit.iso` images (NKit keeps
//! the file system table consistent with the image, so files read the same way).

use anyhow::{bail, Context, Result};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// One file or directory in the disc's file system table.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Full path from the disc root, using `/` separators, without a leading slash.
    pub path: String,
    pub is_dir: bool,
    /// Byte offset within the image (files only).
    pub offset: u64,
    /// Size in bytes (files only).
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct Header {
    pub game_id: String,
    pub title: String,
    pub dol_offset: u64,
    pub fst_offset: u64,
    pub fst_size: u64,
}

pub struct Disc {
    file: File,
    pub header: Header,
    pub entries: Vec<Entry>,
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(b[at..at + 4].try_into().unwrap())
}

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

impl Disc {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let mut file = File::open(path).with_context(|| format!("open {}", path.display()))?;
        let mut hdr = vec![0u8; 0x440];
        file.read_exact(&mut hdr)?;
        if be32(&hdr, 0x1c) != 0xC233_9F3D {
            bail!(
                "{} is not a GameCube disc image (bad magic)",
                path.display()
            );
        }
        let header = Header {
            game_id: cstr(&hdr[0..6]),
            title: cstr(&hdr[0x20..0x400]),
            dol_offset: be32(&hdr, 0x420) as u64,
            fst_offset: be32(&hdr, 0x424) as u64,
            fst_size: be32(&hdr, 0x428) as u64,
        };
        let mut disc = Disc {
            file,
            header,
            entries: Vec::new(),
        };
        disc.entries = disc.read_fst()?;
        Ok(disc)
    }

    fn read_fst(&mut self) -> Result<Vec<Entry>> {
        let fst = self.read_at(self.header.fst_offset, self.header.fst_size as usize)?;
        let count = be32(&fst, 8) as usize;
        let strings = &fst[count * 12..];
        let mut out = Vec::with_capacity(count);
        // Stack of (end_index, path_prefix) for open directories.
        let mut dirs: Vec<(usize, String)> = vec![(count, String::new())];
        for i in 1..count {
            while dirs.last().is_some_and(|(end, _)| i >= *end) {
                dirs.pop();
            }
            let e = &fst[i * 12..i * 12 + 12];
            let is_dir = e[0] != 0;
            let name_off = (be32(e, 0) & 0x00FF_FFFF) as usize;
            let name = cstr(&strings[name_off..]);
            let prefix = &dirs.last().unwrap().1;
            let path = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            if is_dir {
                let end = be32(e, 8) as usize;
                dirs.push((end, path.clone()));
                out.push(Entry {
                    path,
                    is_dir,
                    offset: 0,
                    size: 0,
                });
            } else {
                out.push(Entry {
                    path,
                    is_dir,
                    offset: be32(e, 4) as u64,
                    size: be32(e, 8) as u64,
                });
            }
        }
        Ok(out)
    }

    pub fn read_at(&mut self, offset: u64, len: usize) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; len];
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.read_exact(&mut buf)?;
        Ok(buf)
    }

    pub fn find(&self, path: &str) -> Option<&Entry> {
        let path = path.trim_start_matches('/');
        self.entries
            .iter()
            .find(|e| !e.is_dir && e.path.eq_ignore_ascii_case(path))
    }

    pub fn read_file(&mut self, path: &str) -> Result<Vec<u8>> {
        let e = self
            .find(path)
            .with_context(|| format!("no such file on disc: {path}"))?
            .clone();
        self.read_at(e.offset, e.size as usize)
    }

    /// The main executable (DOL). Its size is derived from the DOL section table.
    pub fn read_dol(&mut self) -> Result<Vec<u8>> {
        let h = self.read_at(self.header.dol_offset, 0x100)?;
        let mut end = 0x100u32;
        for i in 0..18 {
            let off = be32(&h, i * 4);
            let size = be32(&h, 0x90 + i * 4);
            end = end.max(off + size);
        }
        self.read_at(self.header.dol_offset, end as usize)
    }
}
