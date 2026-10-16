//! Build revision identity, format-compatible with the C++
//! `_FULLVERSION` string (`vmangos/src/shared/SystemConfig.h:90`):
//! `"<hash> / <date> / <platform>"`, e.g.
//! `2bae5f921f59892d4c41 / 2026-04-12 01:47:07 +0000 / Linux_x64
//! (little-endian)` — the exact string the production binaries embed
//! and log at startup. Every future Rust binary prints
//! [`full_version`] in its startup banner and `--version` output so a
//! running process always maps back to one commit.

/// `REVISION_HASH`: `git rev-parse --short=20 HEAD`, or `"unknown"`.
pub const REVISION_HASH: &str = env!("MANGOS_REVISION_HASH");

/// `REVISION_DATE`: `git show -s --format=%ci`, or the epoch fallback.
pub const REVISION_DATE: &str = env!("MANGOS_REVISION_DATE");

/// Platform suffix in the C++ `_ENDIAN_PLATFORM` shape,
/// `"<Os>_<arch> (little-endian)"` / `"(big-endian)"`.
#[must_use]
pub const fn platform() -> &'static str {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "Linux_x64 (little-endian)"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "Linux_arm64 (little-endian)"
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "Darwin_arm64 (little-endian)"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "Darwin_x64 (little-endian)"
    }
    #[cfg(not(any(
        all(
            target_os = "linux",
            any(target_arch = "x86_64", target_arch = "aarch64")
        ),
        all(
            target_os = "macos",
            any(target_arch = "x86_64", target_arch = "aarch64")
        )
    )))]
    {
        "unknown (little-endian)"
    }
}

/// The `_FULLVERSION` equivalent: `"<hash> / <date> / <platform>"`.
#[must_use]
pub fn full_version() -> String {
    format!("{REVISION_HASH} / {REVISION_DATE} / {}", platform())
}
