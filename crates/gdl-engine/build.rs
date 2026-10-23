//! The build's `git describe` for bug reports (bugreport.rs), when git is there.
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let o = Command::new("git").args(args).output().ok()?;
    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
    (o.status.success() && !s.is_empty()).then_some(s)
}

fn main() {
    if let Some(d) = git(&["describe", "--always", "--dirty"]) {
        println!("cargo:rustc-env=GDL_GIT_DESCRIBE={d}");
    }
    // Commits and checkouts write the HEAD reflog; adds write the index.
    if let Some(dir) = git(&["rev-parse", "--absolute-git-dir"]) {
        for f in ["logs/HEAD", "index"] {
            let p = std::path::Path::new(&dir).join(f);
            if p.exists() {
                println!("cargo:rerun-if-changed={}", p.display());
            }
        }
    }
    println!("cargo:rerun-if-changed=src");
}
