//! The game's text resources: `TEXT/ENGLISH.ROM`, `SCROLL_E.ROM` and
//! `HINTS_E.ROM` (btext.c StringInitSub). A WAD whose chunks hold the strings
//! (TEXT, offsets in TOFF), messages of one or more lines (STRS), named
//! messages (DEFS/SDEF), message lists (LIST/LOFF) and fonts (FONT).

use anyhow::{Context, Result};

#[derive(Clone, Debug, Default)]
pub struct Message {
    /// The message's name, when DEFS names it ("SCROLLSA1").
    pub name: String,
    pub lines: Vec<String>,
    /// Font name from the FONT chunk ("font32"), if any.
    pub font: Option<String>,
    pub scale: f32,
}

#[derive(Clone, Debug, Default)]
pub struct TextRom {
    pub messages: Vec<Message>,
    /// Message lists: indices into `messages`.
    pub lists: Vec<Vec<i32>>,
}

impl TextRom {
    pub fn parse(d: &[u8]) -> Result<TextRom> {
        let u32_at = |o: usize| -> Result<u32> {
            Ok(u32::from_le_bytes(
                d.get(o..o + 4).context("text ROM truncated")?.try_into()?,
            ))
        };
        let cstr = |o: usize| -> String {
            let s = d.get(o..).unwrap_or(&[]);
            let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
            String::from_utf8_lossy(&s[..end]).into_owned()
        };
        let (dir, count) = (u32_at(0)? as usize, u32_at(4)? as usize);
        let mut chunks = std::collections::HashMap::new();
        for i in 0..count {
            let o = dir + i * 16;
            let tag = u32_at(o)?.to_be_bytes();
            let tag = String::from_utf8_lossy(&tag).into_owned();
            chunks.insert(tag, (u32_at(o + 4)? as usize, u32_at(o + 8)? as usize));
        }
        let get = |t: &str| {
            chunks
                .get(t)
                .copied()
                .with_context(|| format!("no {t} chunk"))
        };
        let fonts: Vec<String> = match chunks.get("FONT") {
            Some(&(o, n)) => (0..n).map(|i| cstr(o + i * 20)).collect(),
            None => Vec::new(),
        };
        let (text, _) = get("TEXT")?;
        let (toff, _) = get("TOFF")?;
        let (strs, nstrs) = get("STRS")?;
        let mut names = vec![String::new(); nstrs];
        if let (Some(&(defs, _)), Some(&(sdef, n))) = (chunks.get("DEFS"), chunks.get("SDEF")) {
            for (i, name) in names.iter_mut().enumerate().take(n) {
                *name = cstr(defs + u32_at(sdef + i * 4)? as usize);
            }
        }
        let mut messages = Vec::with_capacity(nstrs);
        for (i, name) in names.into_iter().enumerate() {
            let o = strs + i * 20;
            let lines_n = u32_at(o)? as i32;
            let first = u32_at(o + 4)? as i32;
            let font = u32_at(o + 8)? as i32;
            let scale = f32::from_bits(u32_at(o + 12)?);
            let mut lines = Vec::new();
            for j in 0..lines_n.max(0) {
                let off = u32_at(toff + (first + j) as usize * 4)? as usize;
                lines.push(cstr(text + off).replace("\r\n", "\n"));
            }
            messages.push(Message {
                name,
                lines,
                font: (font >= 0)
                    .then(|| fonts.get(font as usize).cloned())
                    .flatten(),
                scale,
            });
        }
        let mut lists = Vec::new();
        if let (Some(&(list, n)), Some(&(loff, _))) = (chunks.get("LIST"), chunks.get("LOFF")) {
            for i in 0..n {
                let c = u32_at(list + i * 8)? as usize;
                let f = u32_at(list + i * 8 + 4)? as usize;
                lists.push(
                    (0..c)
                        .map(|j| u32_at(loff + (f + j) * 4).map(|v| v as i32))
                        .collect::<Result<_>>()?,
                );
            }
        }
        Ok(TextRom { messages, lists })
    }

    /// A message by name ("SCROLLSA1").
    pub fn named(&self, name: &str) -> Option<&Message> {
        self.messages
            .iter()
            .find(|m| m.name.eq_ignore_ascii_case(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_scrolls_parse() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../game-data/files/Gauntlet/TEXT/SCROLL_E.ROM");
        let Ok(d) = std::fs::read(root) else { return };
        let rom = TextRom::parse(&d).unwrap();
        let a1 = rom.named("SCROLLSA1").unwrap();
        assert_eq!(a1.lines[0], "Water holds passage to your dreams.");
        assert_eq!(a1.font.as_deref(), Some("font32"));
        assert!((a1.scale - 0.5).abs() < 1e-6);
    }
}
