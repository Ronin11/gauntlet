//! Saved heroes: the Gauntlet save file on a memory card (memcard.c,
//! player.c PlayerLoadSaveFile / PlayerWriteSaveFile).
//!
//! The original keeps one "Gauntlet - Dark Legacy" file on the card with a
//! directory of 8 entries (`DirEntry {size, time, name[8]}`, "Empty" when
//! unused) and one `PlayerSave` image per entry: the hero's name, last class
//! and colour, and per class (16) the experience, health and attribute bonuses
//! (`atts[c]`), the play stats (`stats[c]`) and the inventory and progress
//! block (`stuff[c]`: potions, keys, rune_stones, rune_stones2, level_masks,
//! boss attempts, completion1/2 = gargoyle items and crystals, gold,
//! power-ups).
//!
//! Here the card is virtual: always present and formatted in Slot A, kept as
//! one text file. The Gauntlet file on it may not exist yet; the select screen
//! then offers to create it (motion 12) the first time a hero is saved.

use std::path::{Path, PathBuf};

use crate::game::Carried;
use crate::progress::Progress;

/// Entries in the Gauntlet save file (get_vmu_directory's 8 rows).
pub const FILES: usize = 8;
/// saveFileSize(): bytes the Gauntlet file takes on the card.
pub const SAVE_FILE_SIZE: i32 = 128272;
/// CARD_SYSTEM_BLOCK_SIZE: the select screen counts blocks of this many bytes.
pub const BLOCK: i32 = 8192;
/// The virtual card is an empty Memory Card 251.
pub const CARD_BLOCKS: i32 = 251;
/// The name of an unused entry (lbl_803472D8).
pub const EMPTY: &str = "Empty";
/// What the card file is called in the save directory.
pub const CARD_FILE: &str = "memcard_slot_a.sav";
const MAGIC: &str = "GDL-RS MEMCARD";
const VERSION: u32 = 1;

/// The progress half of a class's `stuff[c]` record: what the party had done
/// when the hero was last saved as that class.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClassProgress {
    pub beaten: [u16; 16],
    pub runes: u16,
    pub shards: u16,
    pub crystals: [i32; 9],
    pub garg_items: [i32; 3],
    pub boss_attempts: [u16; 2],
    pub level_masks: [u16; 4],
    pub legend_items: u16,
    pub banked_runes: u16,
    pub banked_shards: u16,
}

/// Counts that end at -1 once done: done wins, otherwise the larger count.
fn merge_count(a: i32, b: i32) -> i32 {
    if a < 0 || b < 0 {
        -1
    } else {
        a.max(b)
    }
}

impl ClassProgress {
    pub fn from_progress(p: &Progress) -> ClassProgress {
        ClassProgress {
            beaten: p.beaten,
            runes: p.runes,
            shards: p.shards,
            crystals: p.crystals,
            garg_items: p.garg_items,
            boss_attempts: p.boss_attempts,
            level_masks: p.level_masks,
            legend_items: p.legend,
            banked_runes: p.banked_runes,
            banked_shards: p.banked_shards,
        }
    }

    /// OR the masks together; crystals and gargoyle items take the larger
    /// count, with -1 (done) winning.
    pub fn merge(&mut self, o: &ClassProgress) {
        for (a, b) in self.beaten.iter_mut().zip(&o.beaten) {
            *a |= b;
        }
        self.runes |= o.runes;
        self.shards |= o.shards;
        for (a, b) in self.crystals.iter_mut().zip(&o.crystals) {
            *a = merge_count(*a, *b);
        }
        for (a, b) in self.garg_items.iter_mut().zip(&o.garg_items) {
            *a = merge_count(*a, *b);
        }
        for (a, b) in self.boss_attempts.iter_mut().zip(&o.boss_attempts) {
            *a |= b;
        }
        for (a, b) in self.level_masks.iter_mut().zip(&o.level_masks) {
            *a |= b;
        }
        self.legend_items |= o.legend_items;
        self.banked_runes |= o.banked_runes;
        self.banked_shards |= o.banked_shards;
    }

    /// Bring a loaded hero's achievements into the party's progress.
    pub fn merge_into(&self, p: &mut Progress) {
        let mut m = ClassProgress::from_progress(p);
        m.merge(self);
        p.beaten = m.beaten;
        p.runes = m.runes;
        p.shards = m.shards;
        p.crystals = m.crystals;
        p.garg_items = m.garg_items;
        p.boss_attempts = m.boss_attempts;
        p.level_masks = m.level_masks;
        p.legend = m.legend_items;
        p.banked_runes = m.banked_runes;
        p.banked_shards = m.banked_shards;
    }
}

/// One class's slot in a hero's save. `carried` is None for a class never
/// played (player_get_from_save: zero health means clear_player).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClassSave {
    pub carried: Option<Carried>,
    pub progress: ClassProgress,
}

/// A hero's save image (PlayerSave).
#[derive(Clone, Debug, PartialEq)]
pub struct HeroSave {
    pub name: String,
    /// last_alttype / last_color: the class and colour last played.
    pub character: usize,
    pub colour: usize,
    /// Secret classes unlocked: bit (class - 8); 0x100 is Sumner.
    pub class_unlock: u16,
    pub classes: [ClassSave; 16],
}

impl HeroSave {
    pub fn new(name: &str, character: usize, colour: usize) -> HeroSave {
        HeroSave {
            name: name.into(),
            character,
            colour,
            class_unlock: 0,
            classes: Default::default(),
        }
    }

    /// player_store_in_save: the class being played gets the live state, and
    /// the party's progress is ORed into its record.
    pub fn store(&mut self, character: usize, colour: usize, c: &Carried, party: &Progress) {
        self.colour = colour;
        // Sumner (16) is a hidden character: the base character stays saved.
        let Some(slot) = self.classes.get_mut(character) else {
            return;
        };
        self.character = character;
        slot.carried = Some(c.clone());
        slot.progress.merge(&ClassProgress::from_progress(party));
    }

    /// player_get_from_save: a class's state, or a new character for one never
    /// played.
    pub fn carried(&self, class: usize) -> Option<Carried> {
        self.classes.get(class).and_then(|s| s.carried.clone())
    }

    /// leveltot: the levels of all 16 classes added up (the directory entry's
    /// size field).
    pub fn leveltot(&self) -> u32 {
        self.classes
            .iter()
            .map(|s| crate::stats::exp_to_level(s.carried.as_ref().map(|c| c.exp).unwrap_or(0)))
            .sum()
    }
}

/// The memory card in Slot A with its Gauntlet save file.
#[derive(Clone, Debug)]
pub struct MemCard {
    /// Where the card file lives; None keeps the card in memory (tests,
    /// snapshots).
    path: Option<PathBuf>,
    /// The Gauntlet save file exists on the card.
    created: bool,
    entries: [Option<HeroSave>; FILES],
}

impl Default for MemCard {
    fn default() -> MemCard {
        MemCard::in_memory()
    }
}

impl MemCard {
    pub fn in_memory() -> MemCard {
        MemCard {
            path: None,
            created: false,
            entries: Default::default(),
        }
    }

    /// The card kept in `dir` (the file is read now and written on every save).
    pub fn open(dir: Option<&Path>) -> MemCard {
        let mut card = MemCard::in_memory();
        let Some(dir) = dir else {
            return card;
        };
        let path = dir.join(CARD_FILE);
        if let Ok(text) = std::fs::read_to_string(&path) {
            match parse(&text) {
                Ok(entries) => {
                    card.entries = entries;
                    card.created = true;
                }
                Err(e) => {
                    // Kept aside rather than overwritten by a new save file.
                    let bad = path.with_extension("bad");
                    log::warn!(
                        "{}: unreadable save file ({e}); moved to {}",
                        path.display(),
                        bad.display()
                    );
                    let _ = std::fs::rename(&path, &bad);
                }
            }
        }
        card.path = Some(path);
        card
    }

    /// The desktop save directory: $XDG_DATA_HOME/gauntlet-rs, or
    /// ~/.local/share/gauntlet-rs.
    pub fn default_dir() -> Option<PathBuf> {
        if let Some(d) = std::env::var_os("XDG_DATA_HOME").filter(|d| !d.is_empty()) {
            return Some(PathBuf::from(d).join("gauntlet-rs"));
        }
        std::env::var_os("HOME")
            .filter(|d| !d.is_empty())
            .map(|h| PathBuf::from(h).join(".local/share/gauntlet-rs"))
    }

    /// setup_file_entries(pl, 1) > 0: there is a Gauntlet save file.
    pub fn has_save_file(&self) -> bool {
        self.created
    }

    /// saveGetFreeBytes.
    pub fn free_bytes(&self) -> i32 {
        let used = if self.created {
            (SAVE_FILE_SIZE + BLOCK - 1) / BLOCK
        } else {
            0
        };
        (CARD_BLOCKS - used) * BLOCK
    }

    /// MemCardCreateGaunt: a new save file with every entry empty.
    pub fn create(&mut self) -> bool {
        let old = std::mem::take(&mut self.entries);
        self.created = true;
        if self.flush() {
            true
        } else {
            self.entries = old;
            self.created = false;
            false
        }
    }

    /// The directory's names, "Empty" for unused entries.
    pub fn names(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|e| match e {
                Some(h) if h.name.is_empty() || h.name == EMPTY || h.name == "_____" => {
                    "NoName".into()
                }
                Some(h) => h.name.clone(),
                None => EMPTY.into(),
            })
            .collect()
    }

    pub fn is_empty(&self, i: usize) -> bool {
        self.entries.get(i).is_none_or(|e| e.is_none())
    }

    /// saveLoad.
    pub fn load(&self, i: usize) -> Option<HeroSave> {
        if !self.created {
            return None;
        }
        self.entries.get(i)?.clone()
    }

    /// saveSave + add_vmu_file.
    pub fn write(&mut self, i: usize, hero: &HeroSave) -> bool {
        if !self.created || i >= FILES {
            return false;
        }
        let old = self.entries[i].replace(hero.clone());
        if self.flush() {
            true
        } else {
            self.entries[i] = old;
            false
        }
    }

    /// Write the card file (via a temporary file, so a failed write keeps the
    /// old one).
    fn flush(&self) -> bool {
        let Some(path) = &self.path else {
            return true;
        };
        let text = serialize(&self.entries);
        let tmp = path.with_extension("tmp");
        let r = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| std::fs::write(&tmp, text))
            .and_then(|_| std::fs::rename(&tmp, path));
        if let Err(e) = &r {
            log::warn!("{}: {e}", path.display());
        }
        r.is_ok()
    }
}

fn join<T: std::fmt::Display>(v: &[T]) -> String {
    v.iter()
        .map(|x| x.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The card file: a versioned, line-based text format.
///
/// ```text
/// GDL-RS MEMCARD 1
/// entry <0..7>
/// name "<name>"
/// last <class> <colour>
/// unlock <class_unlock>
/// class <0..15>
/// health <f32>   gold <u32>   keys <u32>   potions <u8>...
/// level <u32>    exp <u32>    sumner_exp <u32>
/// bonus <fight> <speed> <armor> <magic>
/// stats <enemies> <generators> <gold_found> <playtime>
/// powerup <time> <kind> <amount> <flags> <state>     (11 lines)
/// beaten <u16 x16>  runes  shards  crystals <x9>  garg <x3>
/// boss <x2>  masks <x4>  legend  banked <runes> <shards>
/// end
/// ```
pub fn serialize(entries: &[Option<HeroSave>; FILES]) -> String {
    let mut s = format!("{MAGIC} {VERSION}\n");
    for (i, h) in entries.iter().enumerate() {
        let Some(h) = h else {
            continue;
        };
        s += &format!("entry {i}\nname \"{}\"\n", h.name.replace('"', ""));
        s += &format!(
            "last {} {}\nunlock {}\n",
            h.character, h.colour, h.class_unlock
        );
        for (c, cs) in h.classes.iter().enumerate() {
            let Some(k) = &cs.carried else {
                continue;
            };
            s += &format!("class {c}\n");
            s += &format!("health {}\ngold {}\nkeys {}\n", k.health, k.gold, k.keys);
            s += &format!("potions {}\n", join(&k.potions));
            s += &format!(
                "level {}\nexp {}\nsumner_exp {}\n",
                k.level, k.exp, k.sumner_exp
            );
            let b = k.bonus;
            s += &format!("bonus {} {} {} {}\n", b.fight, b.speed, b.armor, b.magic);
            let st = k.stats;
            s += &format!(
                "stats {} {} {} {}\n",
                st.enemies_killed, st.generators_destroyed, st.gold_found, st.playtime
            );
            for p in &k.powerups.slots {
                s += &format!(
                    "powerup {} {} {} {} {}\n",
                    p.time, p.kind, p.amount, p.flags, p.state
                );
            }
            let p = &cs.progress;
            s += &format!("beaten {}\n", join(&p.beaten));
            s += &format!("runes {}\nshards {}\n", p.runes, p.shards);
            s += &format!(
                "crystals {}\ngarg {}\n",
                join(&p.crystals),
                join(&p.garg_items)
            );
            s += &format!(
                "boss {}\nmasks {}\n",
                join(&p.boss_attempts),
                join(&p.level_masks)
            );
            s += &format!(
                "legend {}\nbanked {} {}\n",
                p.legend_items, p.banked_runes, p.banked_shards
            );
        }
        s += "end\n";
    }
    s
}

fn nums<T: std::str::FromStr + Default + Copy, const N: usize>(
    rest: &str,
) -> Result<[T; N], String> {
    let mut out = [T::default(); N];
    let mut it = rest.split_whitespace();
    for o in out.iter_mut() {
        let w = it.next().ok_or("too few values")?;
        *o = w.parse().map_err(|_| format!("bad number {w:?}"))?;
    }
    Ok(out)
}

fn word<T: std::str::FromStr>(w: &str) -> Result<T, String> {
    w.parse().map_err(|_| format!("bad number {w:?}"))
}

fn num<T: std::str::FromStr + Default + Copy>(rest: &str) -> Result<T, String> {
    Ok(nums::<T, 1>(rest)?[0])
}

pub fn parse(text: &str) -> Result<[Option<HeroSave>; FILES], String> {
    let mut lines = text.lines();
    let head = lines.next().ok_or("empty file")?;
    let version = head
        .strip_prefix(MAGIC)
        .ok_or("not a gauntlet-rs memory card")?;
    if num::<u32>(version)? > VERSION {
        return Err(format!(
            "save version {} is newer than this game",
            version.trim()
        ));
    }
    let mut entries: [Option<HeroSave>; FILES] = Default::default();
    let mut hero: Option<(usize, HeroSave)> = None;
    let mut class: Option<usize> = None;
    let mut powerup = 0;
    for (n, line) in lines.enumerate() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
        let err = |e: String| format!("line {}: {e}", n + 2);
        if key == "entry" {
            let i: usize = num(rest).map_err(err)?;
            if i >= FILES {
                return Err(err(format!("entry {i} out of range")));
            }
            hero = Some((i, HeroSave::new("", 0, 0)));
            class = None;
            continue;
        }
        if key == "end" {
            let (i, h) = hero
                .take()
                .ok_or_else(|| err("end outside an entry".into()))?;
            entries[i] = Some(h);
            class = None;
            continue;
        }
        let Some((_, h)) = hero.as_mut() else {
            return Err(err(format!("{key} outside an entry")));
        };
        let r = (|| -> Result<(), String> {
            match key {
                "name" => {
                    let r = rest.trim();
                    h.name = r
                        .strip_prefix('"')
                        .and_then(|r| r.strip_suffix('"'))
                        .unwrap_or(r)
                        .into();
                }
                "last" => {
                    let [c, col] = nums::<usize, 2>(rest)?;
                    h.character = c.min(16);
                    h.colour = col & 3;
                }
                "unlock" => h.class_unlock = num(rest)?,
                "class" => {
                    let c: usize = num(rest)?;
                    if c >= 16 {
                        return Err(format!("class {c} out of range"));
                    }
                    h.classes[c].carried = Some(Carried::default());
                    class = Some(c);
                    powerup = 0;
                }
                _ => {
                    let c = class.ok_or(format!("{key} outside a class"))?;
                    let cs = &mut h.classes[c];
                    let k = cs.carried.as_mut().unwrap();
                    let p = &mut cs.progress;
                    match key {
                        "health" => k.health = num(rest)?,
                        "gold" => k.gold = num(rest)?,
                        "keys" => k.keys = num(rest)?,
                        "potions" => {
                            k.potions = rest
                                .split_whitespace()
                                .map(|w| w.parse().map_err(|_| format!("bad potion {w:?}")))
                                .collect::<Result<_, _>>()?
                        }
                        "level" => k.level = num(rest)?,
                        "exp" => k.exp = num(rest)?,
                        "sumner_exp" => k.sumner_exp = num(rest)?,
                        "bonus" => {
                            let [f, s, a, m] = nums::<f32, 4>(rest)?;
                            k.bonus = crate::stats::Atts {
                                fight: f,
                                speed: s,
                                armor: a,
                                magic: m,
                            };
                        }
                        "stats" => {
                            let w: Vec<&str> = rest.split_whitespace().collect();
                            if w.len() < 4 {
                                return Err("too few values".into());
                            }
                            k.stats.enemies_killed = word(w[0])?;
                            k.stats.generators_destroyed = word(w[1])?;
                            k.stats.gold_found = word(w[2])?;
                            k.stats.playtime = word(w[3])?;
                        }
                        "powerup" => {
                            let w: Vec<&str> = rest.split_whitespace().collect();
                            if w.len() < 5 || powerup >= 11 {
                                return Err("bad powerup".into());
                            }
                            let s = &mut k.powerups.slots[powerup];
                            s.time = word(w[0])?;
                            s.kind = word(w[1])?;
                            s.amount = word(w[2])?;
                            s.flags = word(w[3])?;
                            s.state = word(w[4])?;
                            powerup += 1;
                        }
                        "beaten" => p.beaten = nums(rest)?,
                        "runes" => p.runes = num(rest)?,
                        "shards" => p.shards = num(rest)?,
                        "crystals" => p.crystals = nums(rest)?,
                        "garg" => p.garg_items = nums(rest)?,
                        "boss" => p.boss_attempts = nums(rest)?,
                        "masks" => p.level_masks = nums(rest)?,
                        "legend" => p.legend_items = num(rest)?,
                        "banked" => [p.banked_runes, p.banked_shards] = nums(rest)?,
                        // Keys from a later version are skipped.
                        _ => {}
                    }
                }
            }
            Ok(())
        })();
        r.map_err(err)?;
    }
    if hero.is_some() {
        return Err("unterminated entry".into());
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_carried() -> Carried {
        let mut c = Carried::new_character();
        c.health = 812.5;
        c.gold = 1234;
        c.keys = 3;
        c.potions = vec![0, 2, 2];
        c.level = 7;
        c.exp = 9001;
        c.bonus = crate::stats::Atts {
            fight: 10.0,
            speed: 0.0,
            armor: 25.0,
            magic: 0.5,
        };
        c.stats.enemies_killed = 77;
        c.stats.generators_destroyed = 5;
        c.stats.gold_found = 4000;
        c.stats.playtime = 1234.25;
        c.powerups.slots[0] = crate::powerups::Slot {
            time: 30.5,
            kind: crate::powerups::WEAPON,
            amount: 1.0,
            flags: crate::powerups::W_3WAY,
            state: crate::powerups::FRESH,
        };
        c.powerups.slots[3] = crate::powerups::Slot {
            time: -1.0,
            kind: crate::powerups::SPECIAL,
            amount: 3.0,
            flags: crate::powerups::S_PHOENIX,
            state: crate::powerups::OFF,
        };
        c.sumner_exp = 8000;
        c
    }

    fn sample_progress() -> Progress {
        let mut p = Progress::default();
        p.beaten[1] = 0b111;
        p.beaten[13] = 1;
        p.runes = 0x6;
        p.shards = 0x101;
        p.crystals[3] = 2;
        p.crystals[5] = -1;
        p.garg_items = [1, -1, 0];
        p.boss_attempts = [0x2, 0];
        p.level_masks = [1, 0, 4, 0];
        p.legend = 0x10;
        p.banked_runes = 0x2;
        p.banked_shards = 0x1;
        p
    }

    #[test]
    fn round_trip() {
        let mut h = HeroSave::new("CHIP ", 3, 2);
        h.store(3, 2, &sample_carried(), &sample_progress());
        h.store(9, 1, &Carried::new_character(), &Progress::default());
        h.class_unlock = 0x3;
        let mut entries: [Option<HeroSave>; FILES] = Default::default();
        entries[2] = Some(h.clone());
        entries[7] = Some(HeroSave::new("AB", 6, 0));
        let text = serialize(&entries);
        let back = parse(&text).unwrap();
        assert_eq!(back, entries);
        assert_eq!(back[2].as_ref().unwrap().character, 9);
        assert_eq!(back[2].as_ref().unwrap().colour, 1);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse("hello").is_err());
        assert!(parse("GDL-RS MEMCARD 99\n").is_err());
        assert!(parse("GDL-RS MEMCARD 1\nentry 9\n").is_err());
        assert!(parse("GDL-RS MEMCARD 1\nentry 1\nname \"A\"\n").is_err());
    }

    #[test]
    fn card_file_on_disk() {
        let dir = std::env::temp_dir().join(format!("gdl-save-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut card = MemCard::open(Some(&dir));
        assert!(!card.has_save_file());
        assert_eq!(card.free_bytes() / BLOCK, 251);
        assert!(!card.write(0, &HeroSave::new("X", 0, 0)), "no file yet");
        assert!(card.create());
        assert_eq!(card.free_bytes() / BLOCK, 251 - 16);
        assert_eq!(card.names(), vec![EMPTY; FILES]);
        let mut h = HeroSave::new("BOB", 0, 1);
        h.store(0, 1, &sample_carried(), &sample_progress());
        assert!(card.write(4, &h));
        let again = MemCard::open(Some(&dir));
        assert!(again.has_save_file());
        assert_eq!(again.names()[4], "BOB");
        assert!(again.is_empty(3));
        assert_eq!(again.load(4), Some(h));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn merge_progress() {
        let mut party = Progress::default();
        party.beaten[1] = 0b1000;
        party.crystals[3] = 3;
        party.crystals[4] = 1;
        party.garg_items = [2, 0, -1];
        let saved = ClassProgress::from_progress(&sample_progress());
        saved.merge_into(&mut party);
        assert_eq!(party.beaten[1], 0b1111);
        assert_eq!(party.beaten[13], 1);
        assert_eq!(party.crystals[3], 3, "the larger count");
        assert_eq!(party.crystals[4], 1);
        assert_eq!(party.crystals[5], -1, "done wins");
        assert_eq!(party.garg_items, [2, -1, -1]);
        assert_eq!(party.runes, 0x6);
        assert_eq!(party.shards, 0x101);
        assert_eq!(party.boss_attempts, [0x2, 0]);
        assert_eq!(party.level_masks, [1, 0, 4, 0]);
        assert_eq!(party.legend, 0x10);
        assert_eq!((party.banked_runes, party.banked_shards), (0x2, 0x1));
    }

    #[test]
    fn classes_keep_their_own_state() {
        let mut h = HeroSave::new("ANN", 0, 0);
        let mut war = sample_carried();
        war.exp = 5000;
        h.store(0, 0, &war, &Progress::default());
        assert_eq!(h.carried(0).unwrap().exp, 5000);
        assert!(h.carried(5).is_none(), "a class never played is new");
        h.store(16, 0, &war, &Progress::default());
        assert_eq!(h.character, 0, "Sumner isn't saved over the base class");
        assert_eq!(h.leveltot(), crate::stats::exp_to_level(5000) + 15);
    }
}
