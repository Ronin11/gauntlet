//! Game data access: either an extracted directory or the disc image itself,
//! with any number of asset packs layered over it.
//!
//! An asset pack is a directory laid out like the disc's `Gauntlet/` folder
//! (`LEVELS/levelA1/textures.ngc`, `AUDIO/MUSIC.VBK`, ...). Every read looks
//! in the packs first, in order, and falls back to the disc, so a pack can
//! replace any model, texture, sound or level file by name without the disc
//! changing. Packs are the asset side of the "classic / remix" split: the
//! disc alone is the original game, and packs are swapped in and out
//! without a rebuild.

use anyhow::{bail, Context, Result};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub struct GameFs {
    source: Source,
    /// Asset packs, first wins.
    packs: Vec<PathBuf>,
}

enum Source {
    /// Directory containing the disc's `Gauntlet/` folder contents.
    Dir(PathBuf),
    Disc(PathBuf, RefCell<gcdisc::Disc>),
}

/// The environment variable listing asset packs (`:`-separated on Unix,
/// `;` on Windows), applied before any found in `asset-packs/` folders.
pub const PACKS_ENV: &str = "GDL_ASSETS";
/// A folder of this name beside the game data, the executable or in the
/// working directory holds asset packs, one per subdirectory, applied in
/// name order (number them to choose the layering).
pub const PACKS_DIR: &str = "asset-packs";

/// Folders only the engine reads (full-motion video and streamed music):
/// peers with different ones still simulate alike. Lower case, as compared.
const NOT_SIMULATED: [&str; 2] = ["vqmovies/", "streams/"];

impl GameFs {
    /// Open a disc image (`.iso`/`.gcm`) or a directory. A directory may be the
    /// extracted disc root, its `files/` folder, or the `Gauntlet/` folder itself.
    pub fn open(path: &Path) -> Result<Self> {
        if path.is_file() {
            return Ok(GameFs::from(Source::Disc(
                path.to_path_buf(),
                RefCell::new(gcdisc::Disc::open(path)?),
            )));
        }
        for cand in [
            path.to_path_buf(),
            path.join("Gauntlet"),
            path.join("files/Gauntlet"),
        ] {
            if cand.join("LEVELS").is_dir() {
                return Ok(GameFs::from(Source::Dir(cand)));
            }
        }
        bail!(
            "{} does not look like Gauntlet Dark Legacy game data",
            path.display()
        )
    }

    fn from(source: Source) -> Self {
        GameFs {
            source,
            packs: Vec::new(),
        }
    }

    /// Look for a disc image or extracted data inside `dir`.
    pub fn find_in(dir: &Path) -> Option<Self> {
        if let Ok(fs) = GameFs::open(&dir.join("game-data")) {
            return Some(fs);
        }
        let rd = std::fs::read_dir(dir).ok()?;
        for e in rd.flatten() {
            let p = e.path();
            let ext = p
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if matches!(ext.as_str(), "iso" | "gcm") {
                if let Ok(fs) = GameFs::open(&p) {
                    return Some(fs);
                }
            }
        }
        None
    }

    /// Find game data next to the executable or in common places.
    pub fn discover() -> Result<Self> {
        let mut cands: Vec<PathBuf> = Vec::new();
        if let Ok(p) = std::env::var("GDL_DATA") {
            cands.push(p.into());
        }
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf));
        for base in [Some(PathBuf::from(".")), exe_dir].into_iter().flatten() {
            cands.push(base.join("game-data"));
            if let Ok(rd) = std::fs::read_dir(&base) {
                for e in rd.flatten() {
                    let p = e.path();
                    let ext = p
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    if matches!(ext.as_str(), "iso" | "gcm") {
                        cands.push(p);
                    }
                }
            }
        }
        for c in &cands {
            if let Ok(fs) = GameFs::open(c) {
                log::info!("using game data at {}", c.display());
                return Ok(fs);
            }
        }
        bail!("no game data found; pass --data <iso or extracted dir> or set GDL_DATA")
    }

    /// Where the game data is: the disc image, or the extracted folder.
    pub fn data_path(&self) -> PathBuf {
        match &self.source {
            Source::Dir(root) => root.clone(),
            Source::Disc(path, _) => path.clone(),
        }
    }

    /// Layer an asset pack over the data (and over the packs added before
    /// it). A pack that is not a directory is ignored with a warning.
    pub fn add_pack(&mut self, dir: &Path) {
        if !dir.is_dir() {
            log::warn!("asset pack {} is not a directory; ignored", dir.display());
            return;
        }
        if self.packs.iter().any(|p| p == dir) {
            return;
        }
        log::info!("asset pack: {}", dir.display());
        self.packs.push(dir.to_path_buf());
    }

    /// The asset packs in effect, first wins.
    pub fn packs(&self) -> &[PathBuf] {
        &self.packs
    }

    /// The packs a player sets up without flags: those listed in
    /// `GDL_ASSETS`, then every subdirectory of an `asset-packs/` folder
    /// beside the game data, in each of `dirs`, in the working directory
    /// and beside the executable, in name order.
    pub fn add_default_packs(&mut self, dirs: &[PathBuf]) {
        if let Some(list) = std::env::var_os(PACKS_ENV) {
            for p in std::env::split_paths(&list) {
                if !p.as_os_str().is_empty() {
                    self.add_pack(&p);
                }
            }
        }
        let data = self.data_path();
        let mut bases: Vec<PathBuf> = Vec::new();
        bases.extend(data.ancestors().take(4).map(Path::to_path_buf));
        bases.extend(dirs.iter().cloned());
        bases.push(PathBuf::from("."));
        if let Some(exe) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
        {
            bases.push(exe);
        }
        for base in bases {
            let folder = base.join(PACKS_DIR);
            let Ok(rd) = std::fs::read_dir(&folder) else {
                continue;
            };
            let mut subs: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            subs.sort();
            for s in subs {
                self.add_pack(&s);
            }
        }
    }

    /// The game's executable: from the disc image, or a main.dol next to the
    /// extracted files.
    pub fn read_dol(&self) -> Result<Vec<u8>> {
        match &self.source {
            Source::Dir(root) => {
                for dir in root.ancestors().take(4) {
                    let p = dir.join("main.dol");
                    if p.is_file() {
                        return Ok(std::fs::read(p)?);
                    }
                }
                anyhow::bail!("no main.dol near {}", root.display())
            }
            Source::Disc(_, d) => d.borrow_mut().read_dol(),
        }
    }

    /// Read a file by path relative to the `Gauntlet/` folder, e.g.
    /// `LEVELS/levelA1/WORLDS.PS2`: from the first asset pack that has it,
    /// else from the game data.
    pub fn read(&self, rel: &str) -> Result<Vec<u8>> {
        for pack in &self.packs {
            if let Some(p) = resolve_ci(pack, rel) {
                return std::fs::read(&p).with_context(|| format!("read {}", p.display()));
            }
        }
        match &self.source {
            Source::Dir(root) => {
                let p =
                    resolve_ci(root, rel).with_context(|| format!("missing game file {rel}"))?;
                std::fs::read(&p).with_context(|| format!("read {}", p.display()))
            }
            Source::Disc(_, d) => d.borrow_mut().read_file(&format!("Gauntlet/{rel}")),
        }
    }

    /// Which asset pack `rel` comes from, if any.
    pub fn pack_of(&self, rel: &str) -> Option<&Path> {
        self.packs
            .iter()
            .find(|pack| resolve_ci(pack, rel).is_some())
            .map(PathBuf::as_path)
    }

    pub fn exists(&self, rel: &str) -> bool {
        if self
            .packs
            .iter()
            .any(|pack| resolve_ci(pack, rel).is_some())
        {
            return true;
        }
        match &self.source {
            Source::Dir(root) => resolve_ci(root, rel).is_some(),
            Source::Disc(_, d) => d.borrow().find(&format!("Gauntlet/{rel}")).is_some(),
        }
    }

    /// Every file of the game data and the packs, relative to the `Gauntlet/`
    /// folder and sorted without regard to case; one entry for a file a pack
    /// replaces.
    pub fn list_files(&self) -> Vec<String> {
        let mut files: BTreeMap<String, String> = BTreeMap::new();
        let mut add = |rel: String| {
            files.entry(rel.to_ascii_lowercase()).or_insert(rel);
        };
        match &self.source {
            Source::Dir(root) => walk(root, "", &mut add),
            Source::Disc(_, d) => d
                .borrow()
                .entries
                .iter()
                .filter(|e| !e.is_dir)
                .filter_map(|e| e.path.strip_prefix("Gauntlet/"))
                .for_each(|p| add(p.to_string())),
        }
        for pack in &self.packs {
            walk(pack, "", &mut add);
        }
        files.into_values().collect()
    }

    /// A digest of everything the simulation can read: the game files as the
    /// packs leave them, and the executable's tables. Same on any platform,
    /// from a disc image or an extracted folder. Reads every file it covers.
    pub fn content_digest(&self) -> Result<u64> {
        let mut h: u64 = 0xcbf29ce484222325;
        // 8 bytes at a time: a level's worth of files byte by byte is slow.
        let mut put = |bytes: &[u8]| {
            let chunks = bytes.chunks_exact(8);
            let tail = chunks.remainder();
            for c in chunks {
                h ^= u64::from_le_bytes(c.try_into().unwrap());
                h = h.wrapping_mul(0x100000001b3);
                h ^= h >> 29;
            }
            for &b in tail {
                h ^= b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
            h ^= bytes.len() as u64;
            h = h.wrapping_mul(0x100000001b3);
        };
        for rel in self.list_files() {
            let lower = rel.to_ascii_lowercase();
            if NOT_SIMULATED.iter().any(|d| lower.starts_with(d)) {
                continue;
            }
            put(lower.as_bytes());
            put(&self.read(&rel)?);
        }
        // Absent only for test data: a real game can't load without it.
        if let Ok(dol) = self.read_dol() {
            put(&dol);
        }
        Ok(h)
    }

    /// Names of the subdirectories of `rel`, in the game data and the packs.
    pub fn list_dirs(&self, rel: &str) -> Vec<String> {
        let mut out: Vec<String> = match &self.source {
            Source::Dir(root) => subdirs(&root.join(rel)),
            Source::Disc(_, d) => {
                let prefix = format!("Gauntlet/{rel}/").to_ascii_lowercase();
                d.borrow()
                    .entries
                    .iter()
                    .filter(|e| e.is_dir && e.path.to_ascii_lowercase().starts_with(&prefix))
                    .map(|e| e.path[prefix.len()..].to_string())
                    .filter(|n| !n.contains('/'))
                    .collect()
            }
        };
        for pack in &self.packs {
            if let Some(dir) = resolve_ci(pack, rel) {
                for n in subdirs(&dir) {
                    if !out.iter().any(|o| o.eq_ignore_ascii_case(&n)) {
                        out.push(n);
                    }
                }
            }
        }
        out.sort();
        out
    }
}

/// Every file under `dir`, as paths relative to `root` with `/` separators.
/// Dotfiles are the file manager's, not the game's.
fn walk(dir: &Path, prefix: &str, add: &mut impl FnMut(String)) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let rel = format!("{prefix}{name}");
        if e.path().is_dir() {
            walk(&e.path(), &format!("{rel}/"), add);
        } else {
            add(rel);
        }
    }
}

/// `root/rel` with each component matched case-insensitively (disc paths
/// are), or None where a component is missing.
fn resolve_ci(root: &Path, rel: &str) -> Option<PathBuf> {
    let mut p = root.to_path_buf();
    for part in rel.split('/').filter(|s| !s.is_empty()) {
        let exact = p.join(part);
        if exact.exists() {
            p = exact;
            continue;
        }
        let found = std::fs::read_dir(&p)
            .ok()?
            .flatten()
            .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(part))
            .map(|e| e.path())?;
        p = found;
    }
    Some(p)
}

fn subdirs(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("gdl-fs-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(root: &Path, rel: &str, bytes: &[u8]) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    #[test]
    fn the_content_digest_follows_what_the_game_would_read() {
        let data = temp_dir("digest");
        put(&data, "LEVELS/levelA1/WORLDS.PS2", b"worlds");
        put(&data, "VQMOVIES/intro.vq", b"a movie");
        let fs = GameFs::open(&data).unwrap();
        let base = fs.content_digest().unwrap();
        assert_eq!(fs.content_digest().unwrap(), base, "stable");

        // A movie doesn't change the simulation, nor a file manager's litter.
        put(&data, "VQMOVIES/intro.vq", b"another movie");
        put(&data, "LEVELS/.DS_Store", b"litter");
        assert_eq!(fs.content_digest().unwrap(), base);

        // A level file does, on the disc or from a pack.
        let mut packed = GameFs::open(&data).unwrap();
        let pack = temp_dir("digest-pack");
        put(&pack, "levels/LEVELA1/worlds.ps2", b"other worlds");
        packed.add_pack(&pack);
        assert_ne!(
            packed.content_digest().unwrap(),
            base,
            "a pack's replacement"
        );
        assert_eq!(
            packed.list_files().len(),
            fs.list_files().len(),
            "one file, replaced"
        );

        // A pack that replaces a file with the same bytes changes nothing.
        put(&pack, "levels/LEVELA1/worlds.ps2", b"worlds");
        assert_eq!(packed.content_digest().unwrap(), base);

        let extra = temp_dir("digest-extra");
        put(&extra, "ITEMS/NEW/objects.ngc", b"new");
        let mut added = GameFs::open(&data).unwrap();
        added.add_pack(&extra);
        assert_ne!(added.content_digest().unwrap(), base, "a pack's new file");
    }

    #[test]
    fn packs_shadow_the_disc_in_order_and_fall_through() {
        let data = temp_dir("data");
        put(&data, "LEVELS/levelA1/WORLDS.PS2", b"disc worlds");
        put(&data, "LEVELS/levelA1/objects.ngc", b"disc objects");
        put(&data, "AUDIO/MUSIC.VBK", b"disc music");
        let pack1 = temp_dir("pack1");
        put(&pack1, "levels/LEVELA1/worlds.ps2", b"pack1 worlds");
        put(&pack1, "levels/levelX9/objects.ngc", b"pack1 new level");
        let pack2 = temp_dir("pack2");
        put(&pack2, "LEVELS/levelA1/WORLDS.PS2", b"pack2 worlds");
        put(&pack2, "LEVELS/levelA1/objects.ngc", b"pack2 objects");

        let mut fs = GameFs::open(&data).unwrap();
        assert_eq!(
            fs.read("LEVELS/levelA1/WORLDS.PS2").unwrap(),
            b"disc worlds"
        );
        fs.add_pack(&pack1);
        fs.add_pack(&pack2);
        fs.add_pack(&pack2);
        assert_eq!(fs.packs().len(), 2, "a pack is added once");
        // The first pack wins, matched case-insensitively; the second fills in;
        // the disc is the fallback.
        assert_eq!(
            fs.read("LEVELS/levelA1/WORLDS.PS2").unwrap(),
            b"pack1 worlds"
        );
        assert_eq!(
            fs.read("LEVELS/levelA1/objects.ngc").unwrap(),
            b"pack2 objects"
        );
        assert_eq!(fs.read("AUDIO/MUSIC.VBK").unwrap(), b"disc music");
        assert_eq!(
            fs.pack_of("LEVELS/levelA1/WORLDS.PS2"),
            Some(pack1.as_path())
        );
        assert_eq!(fs.pack_of("AUDIO/MUSIC.VBK"), None);
        assert!(
            fs.exists("LEVELS/levelX9/objects.ngc"),
            "a pack can add files"
        );
        assert!(!fs.exists("LEVELS/levelA1/nothing.bin"));
        assert_eq!(
            fs.list_dirs("LEVELS"),
            vec!["levelA1".to_string(), "levelX9".to_string()]
        );
        for d in [data, pack1, pack2] {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}
