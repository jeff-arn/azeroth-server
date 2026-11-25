# 01 — Build and Toolchain Architecture (vmangos)

Scope: `vmangos/CMakeLists.txt` + all subordinate CMake, deps, flags, generated
files, install layout, and a local macOS build probe. Evidence base for a future
C++→Rust migration guide. All citations are `path:line` inside
`/Users/jeffarn/Programming/azeroth-server/`.

## 1. Top-level build facts

| Fact | Value | Citation |
|---|---|---|
| Build system | CMake, `cmake_minimum_required(VERSION 3.1...3.20)`, `project(MaNGOS)` | vmangos/CMakeLists.txt:19-21 |
| C++ standard | C++14, required, extensions OFF | vmangos/CMakeLists.txt:23-25 |
| Build type default | `Release` forced if unset | vmangos/cmake/options.cmake:46-48 |
| Client build | `SUPPORTED_CLIENT_BUILD=CLIENT_BUILD_1_12_1` (5875) | vmangos/cmake/options.cmake:11 |
| Out-of-source | Hard-required (`FATAL_ERROR` if in-source) | vmangos/CMakeLists.txt:36-42 |
| CMakeLists.txt files | 34 total (root 1, src 7, dep 13, contrib 12, tests 1) | `find vmangos -name CMakeLists.txt` |
| First-party sources | 535 `.cpp` + 394 `.h` under `vmangos/src/` | `find vmangos/src` |
| Formatting gate | clang-format, LLVM-based, config `vmangos/.clang-format`; CI + pre-commit `--dry-run -Werror` | vmangos/.clang-format:2-3, .github/workflows/ci.yml (cpp-format job) |
| CI build | Ubuntu only, `main` pushes only: `cmake .. -DCMAKE_BUILD_TYPE=Release`, `cmake --build . -j$(nproc)` | .github/workflows/ci.yml (cpp-build job) |

Options (vmangos/cmake/options.cmake:3-8): `BUILD_FOR_HOST_CPU=ON` (-march=native),
`USE_SCRIPTS=ON`, `USE_EXTRACTORS=OFF`, `USE_REALMMERGE=OFF`,
`ENABLE_MAILSENDER=OFF` (adds libcurl), `ENABLE_CPPTRACE=ON`,
`BUILD_TESTS=OFF` (vmangos/CMakeLists.txt:292), `USE_PCH=ON` on CMake ≥3.16
(vmangos/cmake/options.cmake:19-21).

## 2. Targets and link graph

Subdirectory order: `src` then `dep`, `contrib` only if extractors/realmerge,
`tests` only if BUILD_TESTS (vmangos/CMakeLists.txt:283-296).
`src` adds shared, framework, realmd, game, mangosd, scripts-if-USE_SCRIPTS
(vmangos/src/CMakeLists.txt:25-33).

### Default build: 11 targets (2 executables + 9 static libraries)

First-party static libs (all `STATIC`, folder "Game Libs"):

- `framework` — header-heavy grid/policy/singleton templates, only 3 `.cpp`;
  links `${TBB_LIBRARIES}` unless `USE_STD_MALLOC` (TBB is never find_package'd
  anywhere in the tree — dead legacy path). vmangos/src/framework/CMakeLists.txt:87-100
- `shared` — DB layer, config, logging, DBC stores, crypto glue; PUBLIC include
  dirs (dep/include, framework, MySQL, OpenSSL), PUBLIC threads, PRIVATE
  `cpptrace::cpptrace` if enabled. vmangos/src/shared/CMakeLists.txt:243-285
- `game` — the monolith: 637-line CMakeLists listing every source explicitly
  (no glob); links `shared`, `Detour`. vmangos/src/game/CMakeLists.txt:595-604
- `scripts` — instance/boss scripts, 404-line explicit list; links `game`
  (circular-ish: mangosd links both). vmangos/src/scripts/CMakeLists.txt:339-347

Executables:

- `mangosd` (world server, port 8085): 11 own sources incl. SOAP + RA socket;
  links `game scripts shared framework g3dlite gsoap` + on UNIX
  MySQL/OpenSSL/zlib; link flags `-pthread -rdynamic`, macOS adds
  `-framework Carbon`. vmangos/src/mangosd/CMakeLists.txt:18,82-145
- `realmd` (auth server, port 3724): 9 own sources; links only
  `shared framework` + MySQL/OpenSSL. vmangos/src/realmd/CMakeLists.txt:18,53-85
  — realmd is by far the smaller migration unit: no game/scripts/g3dlite/gsoap.

Bundled dep libs built by default (vmangos/dep/CMakeLists.txt:18-34):

- `g3dlite` STATIC (G3D math/geometry subset, 40 cpp) → system zlib on UNIX.
  vmangos/dep/src/g3dlite/CMakeLists.txt:73-90
- `Detour` STATIC + `Recast` STATIC (recastnavigation; only Detour is linked by
  `game` — Recast is built but consumed only by the mmap generator).
  vmangos/dep/recastnavigation/CMakeLists.txt:22-23,
  vmangos/dep/recastnavigation/Detour/CMakeLists.txt:30
- `gsoap` STATIC (pregenerated SOAP stubs: stdsoap2/soapC/soapServer) → OpenSSL.
  vmangos/dep/gsoap/CMakeLists.txt:24-29
- `cpptrace-lib` (alias `cpptrace::cpptrace`), vendored modern CMake project
  (its own `cmake_minimum_required 3.12`). vmangos/dep/cpptrace/CMakeLists.txt:204-207

### Optional / non-default targets

- `vmangos_tests` (BUILD_TESTS): doctest single-header from `dep/doctest/`;
  links the FULL game stack because bot AI is entangled with Player/Map/World —
  comment says so explicitly. vmangos/tests/CMakeLists.txt:1-6,17-35
- Extractors (USE_EXTRACTORS): `MapExtractor`, `VMapExtractor`, `VMapAssembler`,
  `MoveMapGenerator` + libs `libmpq`, `bz2`, `loadlib`, `vmap`; install to
  `bin/Extractors`. vmangos/contrib/CMakeLists.txt:19-24
- `RealmMerge` (USE_REALMMERGE). vmangos/contrib/CMakeLists.txt:26-28
- Dead/unwired CMake (present on disk, never add_subdirectory'd from root):
  contrib/{model_reader, vmap_reader, git_id, mysql_to_pgsql}, dep/loadlib
  (duplicate of contrib/extractor/loadlib), dep/src/bzip2; dep/src/zlib only on
  WIN32, dep/src/libseh only on MINGW (vmangos/dep/src/CMakeLists.txt:19-25).

### Link graph (default)

```
realmd ──► shared ──► (threads, cpptrace)        framework (leaf, header-mostly)
        └─► framework
mangosd ──► game ──► shared, Detour
        ├─► scripts ──► game        (scripts→game back-edge; static-link order matters)
        ├─► shared, framework
        ├─► g3dlite ──► zlib(system)
        └─► gsoap ──► OpenSSL(system)
        └─► MySQL, OpenSSL, zlib (all system, UNIX)
```

Everything is statically linked into the two daemons; there are no shared
libraries, no plugin .so boundary. A Rust rewrite therefore has no existing
ABI seam to straddle except process boundaries (realmd vs mangosd) and the DB.

## 3. Third-party dependencies: bundled vs system

System (UNIX, REQUIRED): MySQL, OpenSSL, ZLIB via custom find modules
(vmangos/CMakeLists.txt:112-116; finders in vmangos/cmake/find/). CURL only if
ENABLE_MAILSENDER (vmangos/CMakeLists.txt:118-130). **No ACE, no Boost, no
Asio** — networking is hand-rolled in src/framework + src/game/Server; the only
TBB references are the dead `USE_STD_MALLOC` branches above. CI apt line is the
canonical system-dep list: `build-essential cmake git libssl-dev
libmysqlclient-dev libcurl4-openssl-dev zlib1g-dev` (.github/workflows/ci.yml,
cpp-build job).

Bundled in dep/: g3dlite, recastnavigation (Detour+Recast), gsoap (generated
stubs committed), cpptrace, libmpq (extractors only), single-header
nlohmann json (`dep/include/json/json.hpp`), doctest (`dep/doctest/doctest.h`),
header-only mersennetwister/utf8cpp/bzip2/postgre includes under dep/include/.
Windows gets fully prebuilt MySQL/OpenSSL/zlib from `dep/windows/`
(vmangos/CMakeLists.txt:97-109) — irrelevant to a Linux-target Rust port but it
pins ancient `libeay32` naming.

Legacy find modules are a migration-relevant liability: FindMySQL shells out to
`mysql_config --libs_r` and regex-parses flags, with hardcoded MySQL 5.0/5.1
Windows registry paths (vmangos/cmake/find/FindMySQL.cmake:20-62,75-82);
FindOpenSSL hardcodes Homebrew `openssl@1.1` paths (vmangos/cmake/find/FindOpenSSL.cmake).

## 4. Compiler flags and platform settings

UNIX flags (vmangos/cmake/platform/unix/settings.cmake, included via
detect_platform.cmake:15-17): `--no-warnings -fexceptions
-fnon-call-exceptions -pipe` on every config, plus `-march=native` when
BUILD_FOR_HOST_CPU (default ON — binaries are host-CPU-specific, a real
deployment constraint today). Optional GCC_SANITIZE (ASan etc.) and PGO
(PROFILE_GENERATE/USE) branches exist but are off by default. No `-Wall/-Werror`
anywhere; warnings are globally suppressed with `--no-warnings`.
Definitions: `DO_MYSQL` always, `SYSCONFDIR="<prefix>/etc/"` on non-Windows,
`NDEBUG` (Release) / `_DEBUG MANGOS_DEBUG` (Debug) — vmangos/CMakeLists.txt:195-196,256-266.
PCH: `game` and `scripts` precompile ~12 fat headers (WorldSocket.h, Common.h,
ObjectMgr.h…) — a symptom of header entanglement a Rust crate graph must not
replicate. vmangos/src/game/CMakeLists.txt:620-636

macOS-specific: XCode forces `CMAKE_OSX_ARCHITECTURES x86_64` (pre-ARM
assumption, vmangos/CMakeLists.txt:245-251) and both daemons link the
deprecated Carbon framework on APPLE (src/mangosd/CMakeLists.txt:139-141,
src/realmd/CMakeLists.txt:93-95).

## 5. Generated files (configure-time codegen into the SOURCE tree)

Three headers are generated at cmake-configure time and written **into the
source directory**, not the build dir — so even an out-of-tree configure dirties
the checkout (none of the three exist in this pristine checkout today):

- `src/shared/revision.h` from git hash/date (vmangos/CMakeLists.txt:173,
  template cmake/generators/revision.h.in)
- `src/shared/Progression.h` — client-build constants + `SUPPORTED_CLIENT_BUILD`
  substitution (vmangos/CMakeLists.txt:177, cmake/generators/Progression.h.in)
- `src/shared/migrations_list.h` — glob of `sql/migrations/*.sql` baked into
  C arrays per database (vmangos/CMakeLists.txt:180,
  cmake/generators/migrations_list.h.cmake:1-31). The binary thus carries its
  own expected-migration list; a Rust port needs the equivalent (build.rs glob).

Config templates: `src/mangosd/mangosd.conf.dist.in` is configure_file'd
(`@CMAKE_INSTALL_PREFIX@` substitution) and installed to etc/ as
`mangosd.conf.dist` (src/mangosd/CMakeLists.txt:149-150); realmd's `.dist.in`
is installed verbatim, renamed (src/realmd/CMakeLists.txt:102). Committed
examples also live at `vmangos/run/etc/{mangosd,realmd}.conf.example`
(ConfVersion=2025040601). Live `*.conf` files are gitignored and were not read.

## 6. Install layout

Default prefix: if unset/`/usr/local`, forced to `../server` relative to the
build tree (vmangos/CMakeLists.txt:56-59). Layout: `bin/` executables (+
`run-mangosd` bash crash-restart loop, src/mangosd/CMakeLists.txt:147-148),
`etc/` configs, `lib/` (rpath target; only used for Windows DLLs in practice)
— vmangos/CMakeLists.txt:72-86. The repo's live layout mirrors this:
`vmangos/run/bin/` (currently only the committed `run-mangosd` script on this
Mac; binaries/data are gitignored) and `vmangos/run/etc/*.conf.example`.
Production runs under systemd units azeroth-realmd/azeroth-mangosd (CLAUDE.md).

## 7. Local build probe on this Mac (2026-10-15, read-only)

Host: `Darwin 27.0.0 arm64` (Apple Silicon), no Xcode (CommandLineTools only).

Toolchain observed:

```
$ cmake --version        -> /bin/sh: cmake: command not found
$ c++ --version          -> Apple clang version 21.0.0 (clang-2100.3.34.2), arm64-apple-darwin27.0.0
$ g++ --version          -> Apple clang 21.0.0 (same driver)
$ make --version         -> GNU Make 3.81
$ mysql_config --version -> 26.7.0   (brew: mysql 26.7.0_2)
$ mysql_config --libs    -> -L/opt/homebrew/Cellar/mysql/26.7.0_2/lib -lmysqlclient -lz -lzstd -lssl -lcrypto -lresolv
$ openssl version        -> OpenSSL 3.6.5 (brew openssl@3; headers at /opt/homebrew/opt/openssl@3/include/openssl/ssl.h)
$ brew list              -> NO cmake, NO ninja; mysql + openssl@3 present
```

Configure attempt (exact command requested):

```
$ cmake -S vmangos -B /tmp/muse-rust-arch/build -DCMAKE_BUILD_TYPE=Release
/bin/sh: cmake: command not found
EXIT:127
```

**Result: blocked at step zero — CMake is not installed on this machine**
(not on PATH, not in /opt/homebrew/bin, not as CMake.app). No configure, no
single-target build was possible without installing software, which is outside
this read-only probe. The repo was not modified (the three generated headers
in §5 remain absent).

Predicted next blockers once CMake exists (static analysis, unverified):

1. CMake ≥4 compatibility: root declares `3.1...3.20` and subprojects declare
   as low as `2.6...3.20` (dep/recastnavigation/Detour/CMakeLists.txt); CMake 4
   rejects <3.5 minimums — the `...max` range form may save the root but each
   vendored minimum needs checking. Pin CMake 3.x for a first build.
2. Apple Silicon: XCode branch forces x86_64 (vmangos/CMakeLists.txt:249);
   `-march=native` + `-rdynamic` (src/mangosd/CMakeLists.txt:136) and Carbon
   are GNU/linker assumptions Apple clang/ld64 partially rejects.
3. FindMySQL: `mysql_config --libs_r` still works on brew MySQL 26.7 (verified
   above), but the finder's library search paths omit `/opt/homebrew/lib`, so
   it depends entirely on mysql_config parsing succeeding.
4. FindOpenSSL hardcodes `openssl@1.1` brew paths; this host has openssl@3
   (plain `/opt/homebrew/opt/openssl` also listed, so it may resolve).
5. C++14 codebase vs Apple clang 21 / libc++: `--no-warnings` hides warnings
   but not hard errors from removed C++14-era constructs or OpenSSL 3 API
   removals (code predates OpenSSL 3; `libeay32` naming on Windows side).
6. cpptrace on macOS arm64 needs libdwarf/unwind backend selection
   (dep/cpptrace/CMakeLists.txt links libdwarf/libunwind/dbghelp per platform).

## 8. Rust-migration implications (build scope only)

- **Natural crate seams already exist as CMake targets**: framework → shared →
  game → scripts, with realmd depending only on shared+framework. realmd
  (9 sources) is the credible first Rust binary; mangosd is one static blob
  with a scripts→game back-edge that must be broken (trait/registry) in Rust.
- **Cargo replaces**: 34 CMakeLists, 3 custom Find modules, PCH hacks, and the
  `--no-warnings` flag soup. System deps shrink to link-time only if Rust uses
  native-tls/rustls + a pure-Rust MySQL driver — otherwise openssl-sys /
  mysqlclient-sys reproduce exactly the Find-module pain documented above.
- **Vendored C/C++ that needs a decision each**: Detour (runtime navmesh query
  — keep via FFI or port; Recast only needed by offline tools), g3dlite (math
  → glam/nalgebra-class crate), gsoap (SOAP RA interface — drop or reimplement
  the wire protocol), cpptrace (→ backtrace crate), libmpq+zlib+bzip2
  (extractor tools only → flate2/bzip2 crates; tools can stay C++ longest).
- **Codegen parity**: revision.h / Progression.h / migrations_list.h map to a
  build.rs emitting the same constants; the migrations glob is load-bearing —
  the server refuses/knows migrations from a baked-in list, not the filesystem.
- **Build reproducibility**: `-march=native` default means today's production
  binary is host-specific; Rust should set an explicit target-cpu baseline.
  Configure-writes-into-source-tree (§5) disappears with Cargo's OUT_DIR.
- **Tests**: vmangos_tests links the whole game lib (tests/CMakeLists.txt:3-6)
  — in Rust, crate boundaries are the testability fix; porting order should
  extract pure-logic crates (framework equivalents) before entangled game code.
- **Platform**: production is Linux/systemd (CI ubuntu-latest); macOS is dev
  only and currently cannot even configure here (no CMake). A Rust workspace
  builds identically on both, removing the Carbon/x86_64/find-module class of
  macOS breakage entirely. Windows support (dep/windows prebuilt libs) can be
  dropped unless a Windows operator exists.

## 9. Unresolved / not verified

- Actual configure+compile on macOS: impossible without installing CMake
  (read-only constraint). Blockers in §7.1-7.6 are inferred, not observed.
- Linux production build times / warning baseline: CI exists (.github/workflows/ci.yml)
  but was not executed or inspected beyond the workflow file.
- Whether `USE_STD_MALLOC`/TBB is ever set in production builds: no setter found
  in-tree; presumed dead.
- Exact production prefix/flags on the live host (systemd units not read —
  live-system inspection out of scope).
