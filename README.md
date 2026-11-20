# Runtime Arena

A self-hosted judge for small programming contests where participants are
ranked by how **fast** and how **lean** their solutions run.

Participants write code in their own editor and any language, build a Linux
x86-64 executable (or a shebang script), and submit it with the `arena` CLI. The
server runs it against hidden tests in an [isolate](https://github.com/ioi/isolate)
sandbox and keeps live time and memory leaderboards per problem.

> **Status: early development.** The repository layout, CI/CD, images and the
> sandbox are in place; the server and CLI are skeletons. See [PLAN.md](PLAN.md).

## Hosting a contest

Requirements: a Linux host with cgroup v2 (any current distribution), Docker
with Compose. The judge container runs **privileged**; read
[SECURITY.md](SECURITY.md) first.

```sh
git clone https://github.com/hugoocoto/runtime-arena
cd runtime-arena
cp .env.example .env      # set JUDGE_CPUS, see below
docker compose up -d
curl localhost:8080/api/info
```

### Stable timings

- **Pin the judge to CPUs** with `JUDGE_CPUS` in `.env`: one hardware thread per
  physical core, and on hybrid Intel CPUs only performance cores. `lscpu -e=CPU,CORE,MAXMHZ`
  shows them. On an i5-12450H, for example, CPUs 0–7 are 4 P-cores with 2 threads
  each and 8–11 are E-cores, so `JUDGE_CPUS=0,2,4,6`. The same binary measured
  ~0.105 s on a P-core and ~0.163 s on an E-core there.
- Run `scripts/host-tuning.sh` to see settings that reduce noise (CPU governor,
  turbo, SMT, ...), and `sudo scripts/host-tuning.sh --apply` on a dedicated machine.

## Participating

Download `arena` for your platform from the
[releases](https://github.com/hugoocoto/runtime-arena/releases).

Submissions must run on the judge: Linux x86-64, linked against the same libraries.
Build inside the published build image, which shares its base with the judge:

```sh
docker run --rm --platform linux/amd64 -v "$PWD:/work" \
  ghcr.io/hugoocoto/runtime-arena-build-env:latest \
  gcc -O2 -o solution solution.c
```

(`--platform linux/amd64` is needed on ARM machines such as Apple Silicon Macs.)

## Development

```sh
cargo build && cargo test
scripts/sandbox-smoke.sh   # builds the images and checks sandbox verdicts (needs Docker)
```

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[AGPL-3.0-or-later](LICENSE). If you run a modified version for others, you must
offer them its source; the server reports its source URL at `/api/info`.
