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
   arena --version
   ```

   Windows PowerShell:

   ```powershell
   irm https://raw.githubusercontent.com/hugoocoto/runtime-arena/main/install.ps1 | iex
   arena --version
   ```

2. Create an account. Use the HTTPS address and event code announced by the
   organiser. Replace `<domain>` with the host's `ARENA_DOMAIN` from `.env`:

   ```sh
   # LAN deployments using Caddy's private certificate:
   arena trust https://<domain>
   arena signup
   # With a public certificate, skip trust and use:
   # arena signup https://<domain>
   ```

   `arena trust` prints the CA fingerprint, asks for confirmation, and saves
   trust for this server in the CLI config. The projector browser needs the same
   CA installed in its operating-system trust store; the CLI itself needs no
   further setup. Both the server and account are remembered, so every later
   command omits the address.

   Alternatively, create the account at `https://<domain>/join`, then run
   `arena login https://<domain>`.

3. Inspect the round and submit a first Python solution:

   ```sh
   arena round
   arena problem sum --samples samples
   cat > solution.py <<'PY'
   #!/usr/bin/env python3
   import sys

   numbers = map(int, sys.stdin.buffer.read().split())
   next(numbers)
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
with Compose, a DNS name pointing at the host, and ports 80 and 443 reachable
from participants. The judge container runs **privileged**; read
[SECURITY.md](SECURITY.md) first.

```sh
git clone https://github.com/hugoocoto/runtime-arena
cd runtime-arena
cp .env.example .env
$EDITOR .env              # set ARENA_ADMIN_TOKEN, ARENA_JOIN_CODE, ARENA_DOMAIN, JUDGE_CPUS
```

After editing `.env`, start the host:

```sh
chmod 600 .env
scripts/host-preflight.sh
scripts/deploy.sh
```

Compose starts a small Caddy proxy alongside the judge. It obtains and renews a
public certificate automatically with Let's Encrypt. `ARENA_DOMAIN` must resolve
to this host, and ports 80 and 443 must be open. For a private rehearsal,
`ARENA_DOMAIN=localhost` uses Caddy's local CA instead. The organiser's CLI keeps
using `http://localhost:${ARENA_PORT}` through the local-only port; participants
use `https://<domain>`.

For a localhost rehearsal, trust the CA in the CLI before signing up:

```sh
arena trust https://localhost
arena signup https://localhost
```

The projector browser also needs the CA installed in its operating-system trust
store. Export it after Caddy starts with
`docker compose cp caddy:/data/caddy/pki/authorities/local/root.crt arena-local-ca.crt`,
then import `arena-local-ca.crt` into that machine's trusted root certificates.

Open `https://<domain>/` on the projector, and announce
`https://<domain>/join` and the event code (`ARENA_JOIN_CODE`): participants
create their own accounts there or with `arena signup`. Then, with the `arena` CLI
([install it](#installing-the-cli)):

```sh
arena login --admin                           # from this directory: reads the token and server from .env
# from another machine, set ARENA_SERVER=https://<domain> and ARENA_ADMIN_TOKEN in its .env or environment
arena admin problem add problems/sum          # a directory: problem.toml, statement.md, tests/
arena admin round create --name "Round 1" --minutes 60 --freeze 10 sum
arena admin round start 1
arena admin submissions                       # review; `arena admin source <id>` downloads code
arena admin user reset-password alice         # forgotten password: prints a new one, logs out their devices
```

The server checks GitHub for a newer release at startup and every 6 hours. It
only notifies you, in its logs, after `arena login --admin`, and with
`arena admin update`; to update, change `ARENA_VERSION` in `.env`, then run
`scripts/deploy.sh`.
Set `ARENA_UPDATE_CHECK=off` in `.env` to disable the check.

### Reusing or resetting a host

To host another contest on the same deployment, leave the data volume in place,
change `ARENA_JOIN_CODE` if needed, and run `scripts/deploy.sh`. Create and start
a new round; old rounds and accounts remain available.

Remove a draft or finished round, including its submissions, with:

```sh
arena admin round delete <round-id>
```

Running rounds cannot be deleted, and a round with queued submissions must finish
judging first. To erase the entire contest, make a backup first, then reset the
Compose volumes and deploy again. This also creates a new localhost CA:

```sh
scripts/backup.sh /path/to/backups
docker compose down -v
scripts/deploy.sh
```

Run `arena trust https://localhost` again after that reset.

Check the deployment from a second machine with `curl https://<domain>/healthz`.
Before an event, make a consistent data backup with
`scripts/backup.sh /path/to/backups`; it briefly stops judging while the SQLite
database and submissions are archived. Keep backups off the judge host.

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
ARENA_SERVER=https://<domain> ARENA_ADMIN_TOKEN=... ARENA_JOIN_CODE=... scripts/mock-contest.sh
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

Both verify the download against the release's `SHA256SUMS`. The Linux/macOS
installer installs to `/usr/local/bin` and uses `sudo` when needed; set `PREFIX`
to change it, or `VERSION` to pick a release. Windows installs to
`%LOCALAPPDATA%\Programs\arena`.

The Linux/macOS installer also installs the `arena` man page and Bash, Zsh, and
Fish completions under `/usr/local/share`. The release archive's `make install`
path installs the same files.

Or download an archive from the
[releases](https://github.com/hugoocoto/runtime-arena/releases), unpack it, and run
`sudo make install` to install to `/usr/local` (`sudo make uninstall` removes it).
On Windows, `arena completions powershell` prints its completions.

## Participant command reference

For the shortest path, follow the [Participant quickstart](#participant-quickstart).
This section covers the participant commands in more detail. If you have not
joined yet, create an account with the event code at
`https://<domain>/join` or with `arena signup`.

```sh
arena trust https://<domain>          # LAN HTTPS only, once
arena signup                         # once; remembers the server and account
arena round                          # problems and time left
arena problem sum --samples samples  # statement and samples
arena board                          # live boards and charts
```

Log in once on another machine with `arena login https://<domain>`, then use
`arena logout` to end the session on this one. Your password is only sent when
logging in: the CLI keeps a per-device token instead. In scripts, use `--username` and
`--password-stdin`, and `--json` on read commands. Exit codes are listed in
`man arena`.

Submissions must run on the judge: a Linux x86-64 executable linked against the
judge's libraries, or a `#!` script for an interpreter installed there
(`python3`). `arena build` runs your command in the published build image, which
shares its base with the judge; on Apple Silicon it runs under emulation. For
compiled solutions, the command after `--` is run inside that image:

```sh
arena build -- gcc -O2 -o sol solution.c
arena build -- g++ -O2 -std=c++20 -o sol solution.cpp
arena build -- cargo build --release --bin solution
arena build -- zig build-exe solution.zig -O ReleaseFast -femit-bin=sol
```

The Cargo command writes its binary to `target/release/solution`; the other
examples write `sol`. Submit the resulting executable with `arena submit`,
including the source with `--src`.

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
