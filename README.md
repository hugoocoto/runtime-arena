# Runtime Arena

A self-hosted judge for small programming contests where participants are
ranked by how **fast** and how **lean** their solutions run.

Participants write code in their own editor and any language, build a Linux
x86-64 executable (or a shebang script), and submit it with the `arena` CLI. The
server runs it against hidden tests in an [isolate](https://github.com/ioi/isolate)
sandbox and keeps live time and memory leaderboards per problem, in the terminal
(`arena board`) and on a projector page.

> **Status: v0, untested in a real event.** Everything in [PLAN.md](PLAN.md) is
> built and covered by tests; run a mock contest before relying on it.

## Hosting a contest

Requirements: a Linux host with cgroup v2 (any current distribution), Docker
with Compose. The judge container runs **privileged**; read
[SECURITY.md](SECURITY.md) first.

```sh
git clone https://github.com/hugoocoto/runtime-arena
cd runtime-arena
cp .env.example .env      # set ARENA_ADMIN_TOKEN (openssl rand -hex 32), ARENA_JOIN_CODE, JUDGE_CPUS
docker compose up -d
```

Open `http://<host>:8080/` on the projector, and announce `http://<host>:8080/join`
and the event code (`ARENA_JOIN_CODE`): participants create their own accounts
there. Then, with the `arena` CLI (download the executable for your platform
from the [releases](https://github.com/hugoocoto/runtime-arena/releases)):

```sh
arena login --admin                           # on the host, from this directory (reads .env)
# from another machine: arena login --admin http://<host>:8080 <ARENA_ADMIN_TOKEN>
arena admin problem add problems/sum          # a directory: problem.toml, statement.md, tests/
arena admin user add alice --display-name "Alice"   # optional: an account made by you; prints its token once
arena admin round create --name "Round 1" --minutes 60 --freeze 10 sum
arena admin round start 1
arena admin submissions                       # review; `arena admin source <id>` downloads code
```

A problem directory:

```
problem.toml   title, time_limit_ms, memory_limit_mb, samples = ["01"], optional checker
statement.md
tests/01.in 01.out ...
```

`checker` names an executable following the testlib convention
(`checker input output answer`, exit 0 accepts). Without one, output is compared
token by token. See `problems/sum` (run `problems/sum/gen.py` for its large tests).

Before the event, run a mock contest (20 participants submitting at once):

```sh
ARENA_SERVER=http://localhost:8080 ARENA_ADMIN_TOKEN=... scripts/mock-contest.sh
```

### Stable timings

- **Pin the judge to CPUs** with `JUDGE_CPUS` in `.env`: one judge worker per CPU,
  one hardware thread per physical core, and on hybrid Intel CPUs only
  performance cores. `lscpu -e=CPU,CORE,MAXMHZ` shows them. On an i5-12450H, for
  example, CPUs 0–7 are 4 P-cores with 2 threads each and 8–11 are E-cores, so
  `JUDGE_CPUS=0,2,4,6`. The same binary measured ~0.105 s on a P-core and
  ~0.163 s on an E-core there.
- Each accepted test runs 3 times and the best time counts (`ARENA_RUNS_PER_TEST`).
- Run `scripts/host-tuning.sh` to see settings that reduce noise (CPU governor,
  turbo, SMT, ...), and `sudo scripts/host-tuning.sh --apply` on a dedicated machine.

## Participating

Download `arena` for your platform from the
[releases](https://github.com/hugoocoto/runtime-arena/releases).

1. Open `http://<host>:8080/join`, pick a username and password, and enter the
   event code the organiser announced.
2. Log in from your terminal (it asks for the username and password):

```sh
arena login <host>:8080
arena round                         # problems and time left
arena problem sum --samples samples # statement; samples saved as samples/01.in ...
arena build -- gcc -O2 -o sol sol.c # build in the judge's environment (Docker)
arena submit sum ./sol --src sol.c  # source is required; shows each test's result
arena board                         # live boards, time × memory chart, ranks over time
```

Logging in again later, or on another machine, is just `arena login` (the
server is remembered). If the organiser created your account, they give you a
token instead: `arena login <host>:8080 <token>`.

Submissions must run on the judge: a Linux x86-64 executable linked against the
judge's libraries, or a `#!` script for an interpreter installed there
(`python3`). `arena build` runs your command in the published build image, which
shares its base with the judge; on Apple Silicon it runs under emulation.

## Development

```sh
cargo build && cargo test
scripts/sandbox-smoke.sh   # builds the images and checks sandbox verdicts (needs Docker)
scripts/judge-tests.sh     # judge crate tests against a real isolate (needs Docker)
```

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[AGPL-3.0-or-later](LICENSE). If you run a modified version for others, you must
offer them its source; the server reports its source URL at `/api/info` and on
the projector page.
