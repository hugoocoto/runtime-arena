# Contributing

## Workflow

1. Branch from `main` and open a pull request.
2. The **PR title** must follow [Conventional Commits](https://www.conventionalcommits.org/)
   (`feat: ...`, `fix(judge): ...`, `docs: ...`). PRs are squash-merged, so the
   title becomes the commit that [release-please](https://github.com/googleapis/release-please)
   uses to pick the next version and write the changelog.
3. CI must pass: formatting, clippy with warnings denied, tests, cargo-deny
   (licenses and advisories), CLI builds on macOS and Windows, and the sandbox
   verdict check.

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
| `scripts/` | sandbox smoke test, host tuning |

## Toolchain

Rust is pinned in `rust-toolchain.toml`; rustup installs it automatically.
Dependabot does not update that file, so bump it by hand.
