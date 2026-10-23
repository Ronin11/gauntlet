//! The sound library: the AUDATPS2.ROM name table and the VBK banks it
//! points into, decoded to PCM on demand. Pure data, no playback (audio.rs
//! owns the output thread), so the simulation's sound tests can use it.

use crate::fs::GameFs;
use gdl_formats::audio::{parse_sound_names, Vbk, BANKS};
use std::collections::HashMap;
use std::sync::Arc;

/// Music track names for a level, best match first.
pub fn music_candidates(level: &str) -> Vec<String> {
    let code = level.trim_start_matches("level");
    let mut chars = code.chars();
    let world = chars.next().unwrap_or('A');
    let n: String = chars.collect();
    let prefix = match world {
        'A' => "CASTLE",
        'B' => "MOUNT",
        'C' => "DESERT",
        'D' => "FOREST",
        'E' | 'F' => "HELL",
        'G' => "ICE",
        'H' => "CATH",
        'I' => "TOWN",
        'J' => "DREAM",
        'K' => "SKY",
        'L' => "BATT",
        'S' => "SECRET",
        'T' => "tower",
        _ => "CASTLE",
    };
    let mut v = Vec::new();
    for suffix in ["", "A", "_1", "a", "b_1", "a_1", "A_1", "B"] {
        v.push(format!("{prefix}{n}{suffix}"));
    }
    v.push(format!("{prefix}1"));
    v.push(format!("{prefix}1A"));
    v
}

pub type Pcm = Arc<Vec<i16>>;

/// Lazily loaded sound banks and decoded samples.
pub struct SoundLib {
    pub names: HashMap<String, (usize, usize, f32)>,
    pub banks: HashMap<usize, Option<(Vec<u8>, Vbk)>>,
    pub cache: HashMap<(usize, usize), (Pcm, u32)>,
}

impl SoundLib {
    pub fn load(fs: &GameFs) -> Option<Self> {
        let rom = fs.read("AUDIO/AUDATPS2.ROM").ok()?;
        let mut names = parse_sound_names(&rom);
        log::info!("sound effects: {} names", names.len());
        // Sound ids (sndid.h) are bank << 16 | index: `#2b0048` plays one.
        let ids: Vec<_> = names
            .values()
            .map(|&v| (format!("#{:x}", v.0 << 16 | v.1), v))
            .collect();
        names.extend(ids);
        Some(SoundLib {
            names,
            banks: HashMap::new(),
            cache: HashMap::new(),
        })
    }

    pub fn has(&self, name: &str) -> bool {
        self.names.contains_key(name)
    }

    /// Resolve a sound name to its clips and volume.
    pub fn resolve(&mut self, fs: &GameFs, name: &str) -> Option<(Vec<(Pcm, u32)>, f32)> {
        let &(bank, idx, dur) = self.names.get(name)?;
        let entry = self.banks.entry(bank).or_insert_with(|| {
            let file = format!("AUDIO/{}.VBK", BANKS.get(bank)?);
            let bytes = fs.read(&file).ok()?;
            let vbk = Vbk::parse(&bytes)
                .map_err(|e| log::warn!("{file}: {e:#}"))
                .ok()?;
            Some((bytes, vbk))
        });
        let (bytes, vbk) = entry.as_ref()?;
        let def = vbk.sounds.get(idx)?.clone();
        let mut clips = Vec::new();
        for &s in &def.samples {
            let clip = match self.cache.get(&(bank, s)) {
                Some(c) => c.clone(),
                None => {
                    let (pcm, rate) = vbk.decode_sample(bytes, s)?;
                    let c = (Arc::new(pcm), rate);
                    self.cache.insert((bank, s), c.clone());
                    c
                }
            };
            clips.push(clip);
        }
        // Some entries carry unrelated leading words; keep the run of clips (always
        // ending with the final one) whose length best matches the listed duration.
        let len = |c: &[(Pcm, u32)]| {
            c.iter()
                .map(|(p, r)| p.len() as f32 / *r as f32)
                .sum::<f32>()
        };
        let best = (0..clips.len()).min_by(|&a, &b| {
            (len(&clips[a..]) - dur)
                .abs()
                .total_cmp(&(len(&clips[b..]) - dur).abs())
        })?;
        Some((clips[best..].to_vec(), def.volume as f32 / 127.0))
    }
}

/// A request's `@N` volume suffix: (the request, N / 127), 1 without one.
pub fn split_volume(req: &str) -> (&str, f32) {
    match req.rsplit_once('@') {
        Some((name, v)) => match v.parse::<u32>() {
            Ok(v) => (name, v as f32 / 127.0),
            Err(_) => (req, 1.0),
        },
        None => (req, 1.0),
    }
}

/// For testing: which alternative (if any) each sound request resolves to, and its length.
pub fn check_sounds(fs: &GameFs, requests: &[String]) -> Vec<(String, Option<(String, f32)>)> {
    let Some(mut lib) = SoundLib::load(fs) else {
        return Vec::new();
    };
    requests
        .iter()
        .map(|req| {
            let found = split_volume(req).0.split('|').find_map(|alt| {
                let list = alt.strip_prefix("VOICE:").unwrap_or(alt);
                let mut total = 0.0;
                for n in list.split('+') {
                    let (clips, _) = lib.resolve(fs, n)?;
                    total += clips
                        .iter()
                        .map(|(p, r)| p.len() as f32 / *r as f32)
                        .sum::<f32>();
                }
                Some((alt.to_string(), total))
            });
            (req.clone(), found)
        })
        .collect()
}

#[cfg(test)]
mod id_tests {
    #[test]
    fn volume_suffix() {
        assert_eq!(super::split_volume("S_WARN@254"), ("S_WARN", 2.0));
        assert_eq!(super::split_volume("VOICE:S_A+S_B"), ("VOICE:S_A+S_B", 1.0));
    }

    #[test]
    fn sounds_play_by_id() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        let Ok(fs) = crate::fs::GameFs::open(&root) else {
            return;
        };
        // S_MINECARPHIT, S_ROCKBREAKE, and S_WARN at a volume.
        let r = super::check_sounds(
            &fs,
            &["#2b0048".into(), "#370000".into(), "S_WARN@202".into()],
        );
        assert!(
            r.iter().all(|(_, f)| f.as_ref().is_some_and(|f| f.1 > 0.0)),
            "{r:?}"
        );
    }

    /// The select bank's name lines are two-clip calls ("Yellow" + "Dwarf"), so the
    /// bank's sound table must be walked call by call: each class has its own name
    /// clip, and S_WELCOME is its own clip, not the first class name.
    #[test]
    fn select_name_lines_say_each_class() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        let Ok(fs) = crate::fs::GameFs::open(&root) else {
            return;
        };
        let Some(mut lib) = super::SoundLib::load(&fs) else {
            return;
        };
        let mut clip = |name: &str| {
            let (bank, idx, _) = lib.names[name];
            lib.resolve(&fs, name).unwrap();
            let vbk = &lib.banks[&bank].as_ref().unwrap().1;
            vbk.sounds[idx].samples.clone()
        };
        let welcome = clip("S_WELCOME");
        assert_eq!(welcome.len(), 1);
        let mut names = std::collections::HashSet::new();
        for class in &crate::heroes::CLASSES[..16] {
            let mut colours = std::collections::HashSet::new();
            let mut class_clip = None;
            for col in crate::heroes::COLOURS {
                let s = clip(&format!("S_{col}{class}1S"));
                assert_eq!(s.len(), 2, "S_{col}{class}1S {s:?}");
                assert_ne!(s[1], welcome[0]);
                colours.insert(s[0]);
                // The class word is the same clip in every colour.
                assert_eq!(*class_clip.get_or_insert(s[1]), s[1], "{class}");
            }
            assert_eq!(colours.len(), 4, "{class}");
            names.insert(class_clip.unwrap());
        }
        assert_eq!(names.len(), 16, "one name clip per class");
    }
}
