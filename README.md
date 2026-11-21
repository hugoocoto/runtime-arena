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

## Participant quickstart

This is the handout for participants. They do not need Rust or Docker if they
use a `#!` script such as Python.

1. Install `arena`:

   Linux or macOS:

   ```sh
   curl -fsSL https://raw.githubusercontent.com/hugoocoto/runtime-arena/main/install.sh | sh
   export PATH="$HOME/.local/bin:$PATH"  # if `arena` is not found afterwards
   arena --version
   ```

   Windows PowerShell:

   ```powershell
   irm https://raw.githubusercontent.com/hugoocoto/runtime-arena/main/install.ps1 | iex
   arena --version
   ```

2. Create an account. Use the server address and event code announced by the
   organiser:

   ```sh
   arena signup http://<host>:8080
   ```

   Alternatively, create the account at `http://<host>:8080/join`, then run
   `arena login http://<host>:8080`.

3. Inspect the round and submit a first Python solution:

   ```sh
   arena round
   arena problem sum --samples samples
   cat > solution.py <<'PY'
   #!/usr/bin/env python3
   import sys

   numbers = map(int, sys.stdin.buffer.read().split())
   next(numbers)                         # n
   print(sum(numbers))
   PY
   arena submit sum solution.py --src solution.py
   arena board
   ```

   The board ranks accepted solutions by CPU time and peak memory. For C, C++,
   Rust, or another compiled language, use `arena build` only if Docker is
   installed locally; the Python path above is the quickest first submission.

## Hosting a contest

Requirements: a Linux host with cgroup v2 (any current distribution), Docker
with Compose. The judge container runs **privileged**; read
[SECURITY.md](SECURITY.md) first.

```sh
git clone https://github.com/hugoocoto/runtime-arena
cd runtime-arena
cp .env.example .env      # set ARENA_ADMIN_TOKEN (openssl rand -hex 32), ARENA_JOIN_CODE, JUDGE_CPUS
chmod 600 .env
scripts/host-preflight.sh
docker compose up -d
```

Open `http://<host>:8080/` on the projector, and announce `http://<host>:8080/join`
and the event code (`ARENA_JOIN_CODE`): participants create their own accounts
there or with `arena signup`. Then, with the `arena` CLI ([install it](#installing-the-cli)):

```sh
arena login --admin                           # from this directory: reads the token and server from .env
# from another machine, set ARENA_SERVER=http://<host>:8080 and ARENA_ADMIN_TOKEN in its .env or environment
arena admin problem add problems/sum          # a directory: problem.toml, statement.md, tests/
arena admin round create --name "Round 1" --minutes 60 --freeze 10 sum
arena admin round start 1
arena admin submissions                       # review; `arena admin source <id>` downloads code
arena admin user reset-password alice         # forgotten password: prints a new one, logs out their devices
```

The server checks GitHub for a newer release at startup and every 6 hours. It
only notifies you, in its logs, after `arena login --admin`, and with
`arena admin update`; to update, run `docker compose pull && docker compose up -d`.
Set `ARENA_UPDATE_CHECK=off` in `.env` to disable the check.

Each network address is limited to 10 failed logins per account and 100 failed
logins or wrong event codes per 15 minutes, 300 new accounts per 15 minutes, 200
requests per second (`ARENA_RATE_LIMIT` in `.env`, 0 disables it) and 512 open
connections. Behind a reverse proxy the server sees every client as the proxy's
address, so these limits are shared.

In a terminal, any argument you leave out is asked for, so `arena admin round create`
alone walks you through it. In scripts a missing argument is an error.

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
ARENA_SERVER=http://localhost:8080 ARENA_ADMIN_TOKEN=... ARENA_JOIN_CODE=... scripts/mock-contest.sh
```

### Stable timings

- **Pin the judge to CPUs** with `JUDGE_CPUS` in `.env`: one judge worker per CPU,
  one hardware thread per physical core, and on hybrid Intel CPUs only
  performance cores. `lscpu -e=CPU,CORE,MAXMHZ` shows them. On an i5-12450H, for
  example, CPUs 0–7 are 4 P-cores with 2 threads each and 8–11 are E-cores, so
  `JUDGE_CPUS=0,2,4,6`. The same binary measured ~0.105 s on a P-core and
  ~0.163 s on an E-core there.
- Each accepted test runs 3 times and the best time counts (`ARENA_RUNS_PER_TEST`).
- Times come from the sandbox cgroup in microseconds, so threads and child
  processes are counted in the total CPU time.
- A submission may create up to 64 processes and threads (`ARENA_PROCESSES`), so
  OpenMP, pthreads and multi-process programs work. Multi-node MPI is not
  supported because the sandbox has no network.
- Each box gets a size-limited tmpfs for its files (`ARENA_DISK_MB`, default
  512 MiB), so a runaway program cannot fill the host disk.
- Run `scripts/host-tuning.sh` to see settings that reduce noise (CPU governor,
  turbo, SMT, ...), and `sudo scripts/host-tuning.sh --apply` on a dedicated machine.

## Installing the CLI

On Linux and macOS:

```sh
curl -fsSL https://raw.githubusercontent.com/hugoocoto/runtime-arena/main/install.sh | sh
```

On Windows, in PowerShell:

```powershell
irm https://raw.githubusercontent.com/hugoocoto/runtime-arena/main/install.ps1 | iex
```

Both verify the download against the release's `SHA256SUMS` and usually need no
admin rights. They install to `~/.local/bin` (`%LOCALAPPDATA%\Programs\arena` on
Windows); set `PREFIX` to change it, or `VERSION` to pick a release.

Or download an archive from the
[releases](https://github.com/hugoocoto/runtime-arena/releases), unpack it, and run
`sudo make install` to install to `/usr/local` (`sudo make uninstall` removes it).
On Linux and macOS you also get the man pages (`man arena`) and bash, zsh and fish
completions. On Windows, `arena completions powershell` prints its completions.

## Participating

For the shortest path, follow [Participant quickstart](#participant-quickstart).
The full command reference is below. Create your account with the event code
the organiser announced (or on `http://<host>:8080/join`).

```sh
arena signup http://<host>:8080
arena round                         # problems and time left
arena problem sum --samples samples # statement; samples saved as samples/01.in ...
arena build -- gcc -O2 -o sol sol.c # build in the judge's environment (Docker)
arena submit sum ./sol --src sol.c  # source is required; shows each test's result
arena board                         # live boards, time × memory chart, ranks over time
```

Log in on another machine with `arena login <host>:8080`, and end the session on
this one with `arena logout`. Your password is only sent when logging in: the CLI
keeps a per-device token instead. In scripts, use `--username` and
`--password-stdin`, and `--json` on read commands. Exit codes are listed in
`man arena`.

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
