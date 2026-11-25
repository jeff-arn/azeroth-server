//! Compiles the vendored Detour sources (the same patched copy
//! mangosd links) plus the C shim into a static library.
//!
//! Source location: `MANGOS_DETOUR_SRC` if set, else the in-repo
//! `vmangos/dep/recastnavigation/Detour` relative to this crate.
//! Fails with an actionable message when the headers are absent
//! (e.g. building the crate outside this repository checkout).

use std::path::PathBuf;

fn detour_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("MANGOS_DETOUR_SRC") {
        return PathBuf::from(dir);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("../../../vmangos/dep/recastnavigation/Detour")
}

fn main() {
    let dir = detour_dir();
    let include = dir.join("Include");
    let source = dir.join("Source");
    assert!(
        include.join("DetourNavMesh.h").exists(),
        "vendored Detour headers not found under {}; set MANGOS_DETOUR_SRC to the \
         recastnavigation/Detour directory (in this repo: vmangos/dep/recastnavigation/Detour)",
        dir.display()
    );

    let detour_sources = [
        "DetourAlloc.cpp",
        "DetourAssert.cpp",
        "DetourCommon.cpp",
        "DetourNavMesh.cpp",
        "DetourNavMeshQuery.cpp",
        "DetourNode.cpp",
    ];

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .opt_level(2)
        .include(&include)
        .include("shim");
    if !build.get_compiler().is_like_msvc() {
        // Path bits in goldens must not depend on FMA contraction;
        // the oracle harness is compiled with the same flag.
        build.flag("-ffp-contract=off");
        build.flag("-std=c++14");
        // Vendored Detour memsets dtMeshTile arrays (by design).
        build.flag_if_supported("-Wno-nontrivial-memcall");
    }
    for src in detour_sources {
        build.file(source.join(src));
    }
    build.file("shim/detour_shim.cpp");
    build.compile("mangos_detour_shim");

    println!("cargo:rerun-if-env-changed=MANGOS_DETOUR_SRC");
    for src in detour_sources {
        println!("cargo:rerun-if-changed={}", source.join(src).display());
    }
    println!("cargo:rerun-if-changed=shim/detour_shim.cpp");
    println!("cargo:rerun-if-changed=shim/detour_shim.h");
}
