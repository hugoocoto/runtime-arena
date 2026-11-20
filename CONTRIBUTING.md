# Contributing

## Workflow

1. Branch from `main` and open a pull request.
2. The **PR title** must follow [Conventional Commits](https://www.conventionalcommits.org/)
   (`feat: ...`, `fix(judge): ...`, `docs: ...`). PRs are squash-merged, so the
   title becomes the commit that [release-please](https://github.com/googleapis/release-please)
   uses to pick the next version and write the changelog.
3. CI must pass: formatting, clippy with warnings denied, tests, cargo-deny
   (licenses and advisories), CLI builds on macOS and Windows, the sandbox
   verdict check and the judge's sandbox tests.

   Plain `cargo test` skips the judge tests that need isolate (they print
   "skipped"). Run them with `scripts/judge-tests.sh`, which builds a test image
   and runs it privileged.

## Releases

release-please keeps a "chore(main): release X.Y.Z" PR up to date. Merging it
tags the release, then the release workflow attaches CLI binaries for Linux,
macOS and Windows and pushes the `server` and `build-env` images to GHCR.

## Layout

| Path | Contents |
|---|---|
| `crates/protocol` | API types shared by server and CLI |
| `crates/judge` | isolate runner |
| `crates/server` | HTTP/WebSocket server |
| `crates/cli` | `arena` CLI |
| `docker/` | Dockerfile (`server`, `build-env` targets) and entrypoint |
| `tests/sandbox` | programs and checks for the sandbox verdict test |
| `crates/judge/tests` | judge tests against real isolate, with fixture problems |
| `crates/server/migrations` | SQLite schema migrations |
| `crates/server/tests`, `crates/cli/tests` | API, queue, WebSocket and CLI tests with a fake judge |
| `web/` | projector page, built into the server binary |
| `problems/` | example problem with reference solutions |
| `scripts/` | sandbox smoke test, judge tests in Docker, mock contest, sqlx query data, host tuning |

## Database queries

The server uses sqlx's compile-checked queries. CI and the Docker build check
them against the committed `.sqlx/` directory (`SQLX_OFFLINE=true`), so after
changing a query or a migration run `scripts/sqlx-prepare.sh` (needs
`cargo install --locked sqlx-cli --no-default-features --features sqlite`) and
commit `.sqlx/`. A build that says "no cached data for this query" means it is
out of date.

## Toolchain

Rust is pinned in `rust-toolchain.toml`; rustup installs it automatically.
Dependabot does not update that file, so bump it by hand.
