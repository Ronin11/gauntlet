//! Game data access: either an extracted directory or the disc image itself.

use anyhow::{bail, Context, Result};
use std::cell::RefCell;
use std::path::{Path, PathBuf};

pub enum GameFs {
    /// Directory containing the disc's `Gauntlet/` folder contents.
    Dir(PathBuf),
    Disc(RefCell<gcdisc::Disc>),
}

impl GameFs {
    /// Open a disc image (`.iso`/`.gcm`) or a directory. A directory may be the
    /// extracted disc root, its `files/` folder, or the `Gauntlet/` folder itself.
    pub fn open(path: &Path) -> Result<Self> {
        if path.is_file() {
            return Ok(GameFs::Disc(RefCell::new(gcdisc::Disc::open(path)?)));
        }
        for cand in [
            path.to_path_buf(),
            path.join("Gauntlet"),
            path.join("files/Gauntlet"),
        ] {
            if cand.join("LEVELS").is_dir() {
                return Ok(GameFs::Dir(cand));
            }
        }
        bail!(
            "{} does not look like Gauntlet Dark Legacy game data",
            path.display()
        )
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

    /// Read a file by path relative to the `Gauntlet/` folder, e.g. `LEVELS/levelA1/WORLDS.PS2`.
    pub fn read(&self, rel: &str) -> Result<Vec<u8>> {
        match self {
            GameFs::Dir(root) => {
                // Disc paths are case-insensitive; resolve each component.
                let mut p = root.clone();
                for part in rel.split('/') {
                    let exact = p.join(part);
                    if exact.exists() {
                        p = exact;
                        continue;
                    }
                    let found = std::fs::read_dir(&p)
                        .with_context(|| format!("read dir {}", p.display()))?
                        .flatten()
                        .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(part))
                        .map(|e| e.path());
                    p = found.with_context(|| format!("missing game file {rel}"))?;
                }
                std::fs::read(&p).with_context(|| format!("read {}", p.display()))
            }
            GameFs::Disc(d) => d.borrow_mut().read_file(&format!("Gauntlet/{rel}")),
        }
    }

    pub fn exists(&self, rel: &str) -> bool {
        match self {
            GameFs::Dir(_) => self.read(rel).is_ok(),
            GameFs::Disc(d) => d.borrow().find(&format!("Gauntlet/{rel}")).is_some(),
        }
    }

    /// Names of the subdirectories of `rel`.
    pub fn list_dirs(&self, rel: &str) -> Vec<String> {
        let mut out: Vec<String> = match self {
            GameFs::Dir(root) => std::fs::read_dir(root.join(rel))
                .map(|rd| {
                    rd.flatten()
                        .filter(|e| e.path().is_dir())
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default(),
            GameFs::Disc(d) => {
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
        out.sort();
        out
    }
}
