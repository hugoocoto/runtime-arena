# Runtime Arena — Plan

A small (~20 people) competitive-programming event platform. Participants write
code in their own editor and language, build a **Linux x86-64 executable** (or a
shebang script) locally, and submit it with a CLI. The server judges it against
hidden tests and ranks by **execution time** and **memory**.

## Decisions

| Area | Decision |
|---|---|
| Evaluation | Server-side. The CLI uploads; the server runs hidden tests in a sandbox. |
| Accepted submissions | Linux x86-64 ELF executables, or scripts with a shebang for interpreters installed on the judge. |
| Source code | Required with every submission. Stored for review only, never executed. |
| Cross-platform builds | A published Docker build image matching the judge; `arena build` runs the user's build command inside it. |
| Sandbox | isolate 2.0 (cgroup v2). |
| Rankings | Two per problem: a **time** board and a **memory** board. |
| Overall standings | **Left open.** Raw per-test results are stored and boards are computed on demand by a pluggable scoring function, chosen per round. v1 ships per-problem boards only. |
| Identity | Admin-issued tokens; `arena login <token>`. |
| Admin | Same CLI with an admin token (`arena admin ...`). |
| Language | Rust for server and CLI. |
| CLI views | Live time and memory leaderboards, time-vs-memory scatter, rank over time, own submission history. |
| Web | Read-only projector page (leaderboards and timer). |
| Contest model | A round has several problems, all visible at start, solvable in any order. |

## Structure

```
runtime-arena/
├── Cargo.toml        (workspace)
├── crates/
│   ├── protocol/     shared API types (serde)
│   ├── judge/        isolate runner library
│   ├── server/       axum API + WebSocket + queue + scoring
│   └── cli/          `arena` binary (ratatui)
├── web/              projector page
├── docker/           Dockerfile (server + build-env targets share a base), entrypoint
├── tests/sandbox/    sandbox verdict test programs
├── scripts/          sandbox smoke test, host tuning
└── problems/         example problem directories
```

- **Deployment**: `docker compose up` with a privileged container and a private
  cgroup namespace; the entrypoint sets up the cgroup tree for isolate
  (`docker/entrypoint.sh`). Verified on cgroup v2 (kernel 7.2) on 2026-11-20.
- **Repo**: github.com/hugoocoto/runtime-arena, AGPL-3.0-or-later, branch `main`.
- **CI/CD**: GitHub Actions (pinned by SHA): fmt, clippy, tests, cargo-deny, CLI
  builds on macOS/Windows, sandbox verdict test in compose. release-please on
  Conventional Commits (PR titles enforced); releasing builds CLI binaries for
  Linux/macOS (x86-64, arm64) and Windows x86-64 and pushes the `server` and
  `build-env` images (amd64) to GHCR. Dependabot for cargo, actions and docker.
- **Protocol**: REST/JSON for actions, multipart upload (binary + source archive),
  one WebSocket for live events (also used by the projector page).
- **Database**: SQLite via sqlx (migrations, async).
- **Storage**: `data/submissions/<id>/{solution,source}` and
  `data/problems/<slug>/v<N>/` on disk; the database records versions and paths.

## Judging

- In-process worker pool: N workers, each pinned to a dedicated CPU core.
- Tests of a submission run in sequence; each hidden test runs **K times**, and
  the lowest CPU time is kept. Peak memory is the program's peak RSS.
- Checker: token-based comparison by default; a problem may ship a custom checker
  executable.
- Per-user submission cooldown, configurable per round.
- Stored per test: verdict, CPU time, wall time, peak memory (the program's peak
  RSS; the cgroup enforces the limit but its usage includes file cache).

Board defaults (open to change):
- Time metric = max CPU time across tests; memory metric = max peak memory across tests.
- Ties are broken by the other metric, then by the earliest submission.
- Best time and best memory may come from different accepted submissions.

## Server

Decided 2026-11-20. Built in three PRs: **A** schema, auth and admin API;
**B** submissions, queue and workers; **C** WebSocket, boards and freeze.

- **Auth**: `Authorization: Bearer <token>`. The admin token is `ARENA_ADMIN_TOKEN`
  (required, ≥ 24 characters, or the server refuses to start). Participant tokens
  are 256-bit random (`arena_…`), shown once, stored as SHA-256.
- **Rounds**: created as drafts, started manually, end by themselves after their
  duration; the admin can stop early or extend. One round runs at a time.
  Submissions are accepted only while a round runs. Optional `freeze_minutes`
  hides board changes near the end (participants still see their own verdicts).
  Per-round `cooldown_seconds` between a participant's submissions.
- **Problems**: identified by slug, reusable across rounds. Uploaded as `.tar.gz`;
  each upload is a new version directory. Locked once a round using it starts;
  after that only an explicit rejudge.
- **Live updates**: one WebSocket. The CLI follows its own submission's progress
  and result there; boards are sent as full snapshots whenever they change. The
  projector connects without a token and gets only public events.
- **Database**: SQLite via sqlx with compile-checked queries; `.sqlx/` holds the
  query metadata for offline builds (`scripts/sqlx-prepare.sh` regenerates it).

API (part A):

| Method | Path | Who |
|---|---|---|
| GET | `/api/info`, `/healthz` | anyone |
| GET | `/api/me` | participant |
| GET | `/api/rounds/current` | anyone |
| GET | `/api/problems/{slug}` (statement and samples, once its round started) | participant, admin |
| GET, POST | `/api/admin/users` | admin |
| POST | `/api/admin/users/{name}/token` (new token) | admin |
| GET | `/api/admin/problems` | admin |
| PUT | `/api/admin/problems/{slug}` (`.tar.gz` body) | admin |
| GET, POST | `/api/admin/rounds` | admin |
| POST | `/api/admin/rounds/{id}/start`, `/stop`, `/extend` | admin |

## Problem format

```
problems/two-sum/
├── problem.toml   title, time_limit_ms, memory_limit_mb, checker, samples = ["01"]
├── statement.md
└── tests/
    ├── 01.in  01.out
    └── ...
```

Uploaded with `arena admin problem add <dir>`.

## CLI

```
arena login [--admin] <server> <token>
arena whoami | arena round | arena problem <slug> [--samples DIR]
arena build -- <build command>          # inside the build image
arena submit <problem> ./exe --src <file|dir> [--no-wait]
arena submissions [id]
arena board                             # TUI: time / memory / scatter / rank-over-time / mine
arena admin user add|list|reset
arena admin problem add <dir> [--slug] [--rejudge] | list
arena admin round create|list|start|stop|extend
arena admin submissions [--round] | source <id> | rejudge <id>
```

## Build order

1. ~~**isolate check**~~ done: isolate v2.7 in a privileged container gives correct
   verdicts (OK, TLE, MLE via cgroup OOM, SIGSEGV, no network). Timing on a pinned
   P-core: min 0.103 s, median 0.105 s, max 0.128 s over 30 runs, so K≥3 is
   worthwhile. E-cores are ~50% slower; pin to P-cores only.
2. ~~`judge` crate~~ done: run a binary against tests, produce verdict, time and memory.
   Every test runs (the report names each failing test); accepted tests are
   repeated `runs_per_test` times (default 3) and keep the best time and memory;
   each run gets a fresh box. Single process (`--processes=1`). Custom checkers
   use the testlib convention (`checker in out ans`, exit 0 = AC, 1/2 = WA).
3. ~~`server`~~ done: schema and migrations, token auth, admin API for users,
   problems and rounds, submission intake (ELF x86-64 / `#!` checks, cooldown,
   source required), in-process workers (one per `JUDGE_CPUS` entry, box `i` on
   CPU `i`), restart recovery, rejudge, boards with freeze, WebSocket events,
   graceful SIGTERM shutdown. Tested with a fake judge (`cargo test`) and end to
   end in compose with isolate (AC, WA on test 02, TLE, MLE, shebang script).
4. ~~`cli`~~ done: `login`, `whoami`, `round`, `problem`, `build`, `submit` (live
   per-test progress over the WebSocket, polling fallback), `submissions`,
   `admin user|problem|round|submissions|source|rejudge`. Config in
   `~/.config/arena/config.toml` (mode 0600), env overrides.
5. ~~`cli` board TUI~~ done: tabs for the time and memory boards, time × memory
   scatter, rank over time (by CPU time, from
   `GET /api/rounds/{id}/problems/{slug}/accepted`) and your submissions.
6. ~~Projector page~~ done: `web/index.html`, served at `/` from the binary,
   anonymous WebSocket, rotates problems every 20 s.
7. ~~Build image, mock contest, stress test~~ done: `build-env` target and
   `arena build`; `scripts/mock-contest.sh` with the example `problems/sum`.
   On 2026-11-20, 20 participants × 3 simultaneous submissions (60) through
   compose with isolate on 4 pinned P-cores: 40 accepted, 20 wrong answer,
   0 CLI errors, 13.5 s total; boards and projector page correct. A code review
   followed; its fixes are covered by tests (see SECURITY.md).

## Not in v1

Passwords/OAuth, per-language time normalization, anti-cheat recording, persistent
accounts across events, spectator/stream integration, overall standings (left open).
