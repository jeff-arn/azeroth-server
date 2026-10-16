# 08 — Live production droplet (evidence for C++→Rust migration guide)

Scope: live rig at `jeffs-awesome-droplet.tailc8ca7a.ts.net` (Tailscale), inspected
2026-10-16T00:49Z, strictly read-only (`systemctl cat/show/status`, `ps`, `ss`, `ls`, `du`,
`git log/status`, `tail`). No service was restarted, nothing was edited, no `sudo` was used,
and no credential values were read or printed (`.env`/`*.conf` only `stat`-ed or grepped for
non-secret keys). Raw captures: `/tmp/muse-rust-arch/raw-{systemd,ps,layout,bin,cron,mysql}.txt`
on the inspecting workstation.

## 1. Access path (gotchas worth recording)

- The FQDN form fails on this workstation: `ssh jeffs-awesome-droplet.tailc8ca7a.ts.net`
  → `Host key verification failed.` The key on file is stored under the short name only
  (`ssh-keygen -F jeffs-awesome-droplet` matches `~/.ssh/known_hosts:5-7`; server ed25519
  SHA256:`HrbPD+pVmpLO28nnJ0vlpGS9+JnbGAaFBVr8hNOad7Q`). Use the short name `jeffs-awesome-droplet`.
- The OS account is **not** the workstation user: `jeffarn@`, `root@`, `ubuntu@`, `debian@`
  all → `Permission denied (publickey)` with `~/.ssh/id_ed25519`. The correct account is
  `wow`, discoverable from the repo docs: `docs/ARCHITECTURE.md:44`
  (`EnvironmentFile=/home/wow/wow-private-server/.env`). Working invocation:
  `ssh -o BatchMode=yes -o ConnectTimeout=15 wow@jeffs-awesome-droplet '<cmd>'`.
- Server banner: `SSH-2.0-OpenSSH_8.9p1 Ubuntu-3ubuntu0.17` (plain OpenSSH on :22,
  publickey-only; not Tailscale SSH — `tailscale ssh` just wraps ssh and hits the same
  known_hosts FQDN issue). Tailscale sees the node as `100.126.96.57`,
  `tagged-devices`, Linux, active/direct (`104.131.68.176:1315`), `tailscale ping` 7 ms.

## 2. Host identity and headroom

Observed (`hostname`, `uname -a`, `/etc/os-release`, `free -h`, `df -h`, `vmstat 1 3`):

- Ubuntu 22.04.5 LTS, kernel `5.15.0-179-generic`, x86_64, up 32 days (since 2026-09-13),
  load average 0.66/0.57/0.50, 0 interactive users, clock UTC.
- 4 vCPU, CPU model string `DO-Regular` (DigitalOcean shared/general-purpose class).
- RAM 7.8 GiB: 4.1 GiB used, 322 MiB free, 3.3 GiB available; swap 2.0 GiB with **1.3 GiB used**
  (vmstat: 1,326,164 KiB swapped resident, but si/so = 0 and CPU idle 84–92% during sampling —
  historical pressure, not active thrashing).
- Root disk `/dev/vda1` 78 GiB, 38 GiB used, **41 GiB free (48%)**.
- Toolchain present: gcc/g++ 11.4.0, CMake 3.22.1, MySQL client 8.0.46, system python3.
  **No Rust**: `rustc`/`cargo` not on PATH, no `~/.cargo`, no `~/.rustup`.

## 3. systemd service topology

Four production units in `/etc/systemd/system/`, all `Type=simple`, `User=wow`,
`Restart=always` (no `MemoryMax`, sandboxing, or hardening directives anywhere):

| Unit | WorkingDirectory | ExecStart | RestartSec | State at inspection |
|---|---|---|---|---|
| `azeroth-realmd` | `.../vmangos/run/bin` | `.../vmangos/run/bin/realmd` | 5 | running, PID 209408, since 2026-10-13 19:50, NRestarts=24, Tasks=10 |
| `azeroth-mangosd` | `.../vmangos/run/bin` | `.../vmangos/run/bin/mangosd` | 10 | running, PID 258570, since **2026-09-14 08:21**, NRestarts=21, Tasks=73 |
| `azeroth-dashboard` | `.../dashboard` | `dashboard/venv/bin/python3 dashboard/dashboard.py` | 10 | running, PID 2198, since boot 2026-09-13, NRestarts=0 |
| `azeroth-discord-bot` | `.../discord-bot` | `discord-bot/venv/bin/python3 discord-bot/bot.py` | 10 | running, PID 2200, since boot, NRestarts=0 |

(`...` = `/home/wow/wow-private-server`; `After=network.target mysql.service`, mangosd also
`After=azeroth-realmd.service`; the two Python units use
`EnvironmentFile=/home/wow/wow-private-server/.env`.) This matches the documented table at
`docs/ARCHITECTURE.md:46-51`.

**Legacy crash-looping units (operational debt):** `mangosd.service` and `realmd.service`
also exist, pointing at the pre-restructure path `/home/wow/vmangos/run/bin/...`, which no
longer exists (`ls: cannot access '/home/wow/vmangos'`). Both sit in
`activating/auto-restart` with **NRestarts=90,690 (mangosd) and 272,071 (realmd)** and a
fresh `ExecMainStartTimestamp` within the last minute — a permanent fork/exit churn and
journal noise source, and a latent port-conflict if that path ever reappears. A backup of an
old `bot-director.service` exists in `~/backups/pre-restructure/` but no such unit is loaded.

## 4. Live process footprint (baseline for any Rust replacement)

`ps -o pid,user,%cpu,%mem,rss,vsz,nlwp,etime,args` (%CPU is lifetime average):

| Process | RSS | VSZ | Threads (nlwp) | %CPU avg | Elapsed |
|---|---|---|---|---|---|
| `mangosd` (comm `mangosd-main`) | 1,417,508 KiB (~1.35 GiB) | 3,945,752 KiB | **73** | 2.1 | 31d 16h |
| `realmd` (comm `realmd-main`) | 10,420 KiB | 292,408 KiB | 10 | 0.2 | 2d 5h |
| dashboard python3 | 39,812 KiB | 351,892 KiB | 5 | 0.0 | 32d 6h |
| discord-bot python3 | 41,328 KiB | 210,408 KiB | 3 | 0.0 | 32d 6h |
| `mysqld` (system, PID 943) | 1,024,604 KiB | — | 68 | 0.8 | 32d 6h |

systemd cross-check agrees: `azeroth-mangosd` MemoryCurrent=1,458,626,560 B (~1.36 GiB),
CPUUsageNSec=59,810,505,263,000 (≈16.6 CPU-hours over 31.7 days ≈ 2.2% of one core);
realmd MemoryCurrent=3,571,712 B after restart; dashboard ≈44 MB, discord-bot ≈37 MB.

**Co-tenants — the droplet is NOT dedicated to WoW:** an EQEmu (EverQuest) server runs as
the same user (`/tmp/spire-launcher eqemu-server:launcher start`, PID 3391) with **25 `zone`
processes at 40–108 MB RSS each (~1.5 GiB aggregate)**, a second `mysqld` under uid `lxd`
(196 MB, 37 threads — evidently inside an LXD container), plus `dockerd`, `caddy.service`,
apache2, snapd, and `groupme-remindme.service`. WoW's ~1.35 GiB is competing with all of
this inside 7.8 GiB; that is the real memory budget context behind the 1.3 GiB swap use.

## 5. Listening surface

`ss -tlnp` (PIDs visible only for own-user processes without sudo):

- Public (`0.0.0.0`/`*`): **3724** (`realmd-main`), **8085** (`mangosd-main`), **8080**
  (dashboard python3), 22 (sshd), 80/443 (Caddy, presumably reverse-proxying), plus
  `104.131.68.176:5998/5999/9000` (co-tenant, likely EQEmu).
- Loopback only: 3306 + 33060 (WoW MySQL), 3307 (second mysqld), 2222, 3000–3010,
  6000, 7778, 8081, 8082, 8787, 9001, 9500, 2019 (Caddy admin) — co-tenant services whose
  owning PIDs were not visible without sudo (unresolved, §9).
- Tailscale: `100.126.96.57:65511` and an fd7a:… IPv6 listener.

`Server.log` tail confirms 8085 faces the open internet: repeated scanner connections
(e.g. 44.220.188.208, 91.92.41.174 on 2026-10-15) producing
`WorldSocket::DoRecvIncomingData: client sent malformed packet size = 18245, cmd = …` —
i.e. arbitrary untrusted bytes reach the world-socket packet parser continuously.

## 6. Install layout, deployed revision, data assets

Checkout: `/home/wow/wow-private-server` (4.9 GiB total), git remote
`https://github.com/jeff-arn/azeroth-server.git`, branch `fix/database-h-forward-decl`,
HEAD `aaf9b5e 2026-05-26 fix(vmangos): forward-declare QueryNamedResult in Database.h`,
working tree clean (`status --porcelain` = 0 lines). The workstation checkout of the same
repo is at `3589f72` — same commit subject, different hash: `git rev-list --count` shows
exactly 1 commit each way (squash-merge twin). Treat deployed source ≈ workstation source.

Binaries (`stat`/`file`/`ldd` on `vmangos/run/bin/`):

- `mangosd` 19,838,640 B, mtime **2026-04-14**; `realmd` 2,182,912 B, mtime 2026-04-13.
  Both ELF x86-64 PIE, dynamically linked, **not stripped**. The running world server was
  *built ~6 weeks before* the checkout HEAD commit and has run since 2026-09-14 — deploys
  are manual and infrequent, with the build tree kept in place (`vmangos/build` = 719 MB).
- `ldd mangosd` resolves only system libs: `libmysqlclient.so.21`, `libcrypto.so.3`/
  `libssl.so.3` (OpenSSL 3), `libz`, `libzstd`, `libstdc++`, glibc 2.35 family. There is
  no bundled/vendored .so payload to replicate; the platform contract is "Ubuntu 22.04
  system libraries".
- Runtime data in `run/bin/` (the unit WorkingDirectory, and `DataDir = "."` in
  `run/etc/mangosd.conf`): `mmaps` **2.0 GiB**, `vmaps` 574 MiB, `maps` 279 MiB, `5875/`
  (DBC for build 5875) 28 MiB. Configs live in `run/etc/{mangosd,realmd}.conf`
  (111,555 B / 7,177 B, mode 644). Safe keys observed: `WorldServerPort = 8085`,
  `RealmServerPort = 3724`, `BindIP = "0.0.0.0"`, `PlayerLimit = 100`,
  `MaxPlayerLevel = 60`, `RealmID = 1`.
- Logs in WorkingDirectory: `Server.log` only 282,429 B (mtime 2026-10-15 20:35 — the
  last entries are the scanner hits above; no player-session lines in the last 30),
  `LevelUp.log` 12 MiB and `Perf.log` 11 MiB (both frozen 2026-09-14 ~11:17, shortly
  after start), `Network.log` 1.3 MiB, plus a trail of 104-byte `HCR__*.txt` files.
  The world is currently very quiet — near-idle bot/player load at inspection time,
  consistent with the 2.1% lifetime CPU.
- `vmangos/` subtree 3.8 GiB (includes build/ + run assets), `vmangos-database/` 157 MiB,
  `dashboard/` 127 MiB, `discord-bot/` 140 MiB (venvs included).

Note (no values read): `mangosd.conf`/`realmd.conf` are mode **644** and `.env` mode
**664**, i.e. credential-bearing files are readable by any local user. Worth fixing
regardless of migration; a Rust daemon should not inherit this posture.

## 7. MySQL and backups

- `mysql.service` active since boot; server/client 8.0.46 (Ubuntu), PID 943,
  MemoryCurrent ≈ 1.13 GiB, listening on 127.0.0.1:3306 only.
- **Table-level sizes are unresolved (§9):** no `~/.my.cnf`/`~/.mylogin.cnf` exists and
  passwordless login fails (`ERROR 1045 (28000)`); credentials live only in `.env` /
  `*.conf`, which this inspection was barred from reading. `/var/lib/mysql` is not
  readable by user `wow` (`Permission denied`), and sudo was out of scope.
- Backup evidence is strong and current: user crontab
  `0 4 * * * /home/wow/wow-private-server/scripts/backup-db.sh >> .../backups/db/backup.log`.
  `backups/db/` holds 664 MiB = **60 `.sql.gz` files = 15 days × 4 databases**
  (2026-10-01 … 2026-10-15), newest 2026-10-15 04:00, log tail shows clean completion
  and pruning. Per-DB compressed sizes (proxy for live DB scale): `mangos` 32 MiB,
  `characters` 13 MiB, `realmd` 8.8 KiB, `logs` 2.3 KiB. The production data set is
  small — the full character DB compresses to ~13 MiB.

## 8. Rust-migration implications of the live rig

1. **Do not count on building Rust on the droplet.** No toolchain is installed; rustup +
   a workspace `target/` would add GiBs (disk is fine: 41 GiB free), but a parallel
   `cargo build --release` + final link against only ~3.3 GiB available RAM, 1.3 GiB
   already swapped, 4 shared vCPUs, and live game players is exactly how you OOM the
   world server. Current C++ flow *does* build in place (719 MB `build/` in-tree) — the
   migration is the moment to break that habit: build off-box (workstation/CI), ship a
   finished binary, swap `ExecStart` target atomically, keep the old binary for rollback.
2. **Glibc floor is Ubuntu 22.04 (glibc 2.35, OpenSSL 3, `libmysqlclient.so.21`).**
   A `x86_64-unknown-linux-gnu` binary built on a newer distro may not start here.
   Either build in a 22.04 container matching `ldd` above, or go
   `x86_64-unknown-linux-musl` (static; note musl + `libmysqlclient` FFI conflict —
   a pure-Rust MySQL driver removes the `.so.21` dependency entirely, the C++ binary's
   only non-libc hard dependency besides OpenSSL/zlib/zstd).
3. **Drop-in deploy shape already exists.** Units are minimal: `User=wow`, fixed
   `WorkingDirectory`, one `ExecStart` path, `Restart=always`. A Rust `mangosd`/`realmd`
   equivalent needs zero unit redesign to be supervised identically — and the migration
   is a good moment to add the absent hardening (`MemoryMax`, `ProtectSystem`, etc.)
   deliberately rather than by default.
4. **Performance/memory parity bar is explicit and low-pressure:** 1.35 GiB RSS,
   73 OS threads, ~2% of one core lifetime average, `PlayerLimit = 100`. Whatever
   replaces it must hold the same 2.9 GiB of maps/mmaps/vmaps/DBC resident or mmap-ed
   from the *same* `run/bin/` files (`DataDir = "."`) — format-compatible Rust readers
   let both binaries share one asset set during transition; no re-extraction exists in
   the deploy path today.
5. **The network edge is hostile by observation, not theory:** 8085/3724 are bound to
   `0.0.0.0` and internet scanners feed malformed packets into the world socket daily.
   Parser robustness (and fuzzing) of the Rust packet layer is a production requirement,
   and binding-exposure decisions (Tailscale-only vs public) should be re-made consciously.
6. **Deploy hygiene gaps a migration should not inherit:** running binary (2026-04-14)
   lags checkout HEAD (2026-05-26) with no recorded mapping binary→commit (unstripped
   binary + BuildID `8fc715…` is all that ties them); two legacy units crash-loop
   ~363k restarts against a deleted path; credential files are world-readable. A Rust
   pipeline should stamp git revision into `--version`, and cutover should disable
   `mangosd.service`/`realmd.service` before any new daemon claims 3724/8085.
7. **Co-tenancy is a hard scheduling constraint:** EQEmu's 25 zones + a second MySQL +
   Docker/Caddy share the box. Any on-box compile, asset re-extraction, or load test
   contends with two live game servers; parity/load testing of a Rust world server
   needs its own host or a maintenance window, not spare cycles that don't exist.
8. **Data risk is small and well-backed-up:** 15 days of nightly dumps, characters DB
   ~13 MiB gz. A Rust service can be rehearsed against a restored copy of last night's
   dump cheaply; schema compatibility (not data volume) is the migration risk.

## 9. Unresolved (evidence unavailable under read-only/no-secrets rules)

- MySQL per-database/per-table sizes and row counts: passwordless access denied
  (`ERROR 1045`), no defaults/login-path file; obtaining the password would require
  reading `.env`/`mangosd.conf` credential values, which was prohibited.
- `/var/lib/mysql` on-disk size: directory not readable by `wow`; sudo was out of scope.
- Owners/content of other-user loopback listeners (3000–3010, 7778, 8081/8082, 9500, …),
  Caddy site config for :80/:443, and Docker container inventory
  (`docker ps` → permission denied on `/var/run/docker.sock`).
- Why `azeroth-realmd` accumulates restarts (24 total, last 2026-10-13) — journal
  forensics on a production auth server were beyond this pass's remit.
- Exact mapping of the running Apr-14 binary to a git commit (no embedded revision
  string checked; BuildID only).
