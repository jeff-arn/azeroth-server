//! The stamp must have the C++ shape: a 20-char short hash (or the
//! `unknown` fallback), a `%ci` date, and the `hash / date / platform`
//! composition from `SystemConfig.h:90`.

use mangos_build_info::{REVISION_DATE, REVISION_HASH, full_version, platform};

#[test]
fn hash_is_short20_hex_or_unknown() {
    assert!(
        REVISION_HASH == "unknown"
            || (REVISION_HASH.len() == 20 && REVISION_HASH.chars().all(|c| c.is_ascii_hexdigit())),
        "unexpected REVISION_HASH: {REVISION_HASH}"
    );
}

#[test]
fn stamp_matches_cpp_fullversion_shape() {
    // %ci looks like `2026-04-12 01:47:07 +0000` (offset sign varies by
    // committer timezone; the epoch fallback has the same shape).
    assert_eq!(REVISION_DATE.len(), 25, "REVISION_DATE: {REVISION_DATE}");
    let offset = &REVISION_DATE[20..];
    assert!(
        (offset.starts_with('+') || offset.starts_with('-'))
            && offset[1..].chars().all(|c| c.is_ascii_digit()),
        "REVISION_DATE offset: {offset}"
    );
    let full = full_version();
    assert_eq!(
        full,
        format!("{REVISION_HASH} / {REVISION_DATE} / {}", platform())
    );
    assert!(platform().contains("endian)"));
}
