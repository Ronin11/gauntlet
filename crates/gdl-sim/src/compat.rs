//! Can two copies of the game play together?
//!
//! Lockstep peers exchange only inputs, so they must already share everything
//! else: the code, the game files, the rules. Any difference plays on quietly
//! until the games have drifted apart, so peers compare these digests before
//! the first tick and refuse a mismatch with a reason a player can act on.

use crate::fs::GameFs;
use crate::rules::Rules;
use anyhow::Result;
use std::fmt;

/// Debugging switches the simulation reads from the environment. They change
/// what happens, so they are compared like the rules.
const SIM_ENV: [&str; 2] = ["GDL_ITEMS", "GDL_NO_BOSS_CAMERA"];

/// A debugging switch's value. All the simulation's environment reads go
/// through here, so `SIM_ENV` stays complete.
pub(crate) fn sim_env(name: &str) -> Option<String> {
    debug_assert!(SIM_ENV.contains(&name), "add {name} to SIM_ENV");
    std::env::var(name).ok()
}

/// What two peers must agree on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Compat {
    /// The build (`git describe`): the same code simulates the same way.
    pub build: String,
    /// The game files and asset packs in effect.
    pub content: u64,
    /// The rules being played.
    pub rules: u64,
    /// The debugging switches in effect.
    pub env: u64,
}

/// Why two peers can't play together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mismatch {
    Build { ours: String, theirs: String },
    Content,
    Rules,
    Env,
}

impl fmt::Display for Mismatch {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Mismatch::Build { ours, theirs } => {
                write!(
                    f,
                    "different builds of the game: {ours} here, {theirs} there"
                )
            }
            Mismatch::Content => {
                write!(
                    f,
                    "different game files: check the disc and the asset packs"
                )
            }
            Mismatch::Rules => write!(
                f,
                "different rules: classic on one side, remix on the other"
            ),
            Mismatch::Env => write!(
                f,
                "different debugging switches: GDL_ITEMS, GDL_NO_BOSS_CAMERA"
            ),
        }
    }
}

impl std::error::Error for Mismatch {}

impl Compat {
    /// This copy of the game. Reads every simulated file, so ask once.
    pub fn local(fs: &GameFs, rules: &Rules, build: &str) -> Result<Compat> {
        let values = SIM_ENV.map(sim_env);
        Ok(Compat {
            build: build.to_string(),
            content: fs.content_digest()?,
            rules: rules.digest(),
            env: env_digest(&values),
        })
    }

    pub fn check(&self, theirs: &Compat) -> Result<(), Mismatch> {
        if self.build != theirs.build {
            return Err(Mismatch::Build {
                ours: self.build.clone(),
                theirs: theirs.build.clone(),
            });
        }
        if self.content != theirs.content {
            return Err(Mismatch::Content);
        }
        if self.rules != theirs.rules {
            return Err(Mismatch::Rules);
        }
        if self.env != theirs.env {
            return Err(Mismatch::Env);
        }
        Ok(())
    }
}

impl fmt::Display for Compat {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "build {} content {:016x} rules {:016x} env {:016x}",
            self.build, self.content, self.rules, self.env
        )
    }
}

fn env_digest(values: &[Option<String>]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for v in values {
        // A separator, so an unset switch and an empty one differ.
        let text = v
            .as_deref()
            .map_or(vec![0u8], |s| [&[1u8], s.as_bytes()].concat());
        for b in text.into_iter().chain([0xff]) {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compat() -> Compat {
        Compat {
            build: "v1".into(),
            content: 1,
            rules: 2,
            env: 3,
        }
    }

    #[test]
    fn each_difference_is_named() {
        let a = compat();
        assert_eq!(a.check(&a.clone()), Ok(()));
        let other = Compat {
            build: "v2".into(),
            ..compat()
        };
        assert!(matches!(a.check(&other), Err(Mismatch::Build { .. })));
        let other = Compat {
            content: 9,
            ..compat()
        };
        assert_eq!(a.check(&other), Err(Mismatch::Content));
        let other = Compat {
            rules: 9,
            ..compat()
        };
        assert_eq!(a.check(&other), Err(Mismatch::Rules));
        let other = Compat { env: 9, ..compat() };
        assert_eq!(a.check(&other), Err(Mismatch::Env));
    }

    #[test]
    fn rules_presets_do_not_pair() {
        let (classic, remix) = (Rules::classic().digest(), Rules::remix().digest());
        assert_ne!(classic, remix);
    }

    #[test]
    fn the_switches_are_told_apart() {
        let unset = env_digest(&[None, None]);
        assert_ne!(unset, env_digest(&[Some(String::new()), None]));
        assert_ne!(
            env_digest(&[Some("1".into()), None]),
            env_digest(&[None, Some("1".into())])
        );
    }

    /// Everything the simulation reads from the environment must be compared.
    /// Finding the data and the save folder doesn't change what happens.
    #[test]
    fn the_sim_reads_the_environment_only_through_here() {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    out.push(p);
                }
            }
        }
        let mut files = Vec::new();
        walk(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        for f in files {
            let name = f.file_name().unwrap().to_string_lossy().into_owned();
            if matches!(name.as_str(), "compat.rs" | "fs.rs" | "save.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&f).unwrap();
            for needle in ["env::var", "env::vars"] {
                assert!(!text.contains(needle), "{name} reads the environment");
            }
        }
    }
}
