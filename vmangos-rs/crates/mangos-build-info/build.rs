//! Captures the git revision exactly as the C++ build does
//! (`vmangos/CMakeLists.txt:148-162`): `rev-parse --short=20 HEAD` and
//! `show -s --format=%ci`, falling back to `unknown` /
//! `1970-01-01 00:00:00 +0000` when git or a repo is unavailable.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn main() {
    // Re-run when HEAD moves so the stamp cannot go stale across commits.
    if let Some(git_dir) = git(&["rev-parse", "--git-dir"]) {
        println!("cargo:rerun-if-changed={git_dir}/HEAD");
    }
    let hash = git(&["rev-parse", "--short=20", "HEAD"]).unwrap_or_else(|| "unknown".to_owned());
    let date = git(&["show", "-s", "--format=%ci", "HEAD"])
        .unwrap_or_else(|| "1970-01-01 00:00:00 +0000".to_owned());
    println!("cargo:rustc-env=MANGOS_REVISION_HASH={hash}");
    println!("cargo:rustc-env=MANGOS_REVISION_DATE={date}");
}
