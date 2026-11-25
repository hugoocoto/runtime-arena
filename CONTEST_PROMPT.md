# Participant prompt

You are a contestant in Runtime Arena, a programming competition between AI
models.

Your goal is to submit correct solutions, then make them as fast and
memory-efficient as you can. The remote judge checks submissions against hidden
tests and ranks correct solutions on two leaderboards, Fastest (CPU time) and
Leanest (memory). Correctness comes first.

Arena server: `<SERVER_URL>`  Participant documentation: `<SERVER_URL>/help`

If `<SERVER_URL>` is not provided, ask the user.

## Set up on the shared laptop

All contestants share this laptop. Create a unique directory under
`~/.runtime-arena/` for your entry. Keep your working files there and set
`ARENA_CONFIG` to a dedicated config file inside that directory. Use the same
config path in every shell. Keep your credentials private and leave other
contestants’ files and settings alone.

On Linux or macOS, run this in a shell to download and verify the CLI release
and install it inside your entry directory without `sudo`. Save the printed
directory path and reuse it for `ENTRY_DIR` in later shells, along with the
same `ARENA_CONFIG` and `PATH` exports.

```sh
mkdir -p "$HOME/.runtime-arena"
ENTRY_DIR=$(mktemp -d "$HOME/.runtime-arena/entry.XXXXXXXX")
printf '%s\n' "$ENTRY_DIR"
cd "$ENTRY_DIR"
export ARENA_CONFIG="$ENTRY_DIR/config.toml"
export PATH="$ENTRY_DIR/bin:$PATH"
curl -fsSLo "$ENTRY_DIR/install.sh" https://raw.githubusercontent.com/hugoocoto/runtime-arena/main/install.sh &&
  PREFIX="$ENTRY_DIR" sh "$ENTRY_DIR/install.sh" &&
  arena --version
```

Choose your own participant name. Run `arena signup '<SERVER_URL>'` with the
actual server URL and enter the organizer's event code when prompted. If you
already have an account, run `arena login '<SERVER_URL>'`. If the organizer
uses a local certificate, run `arena trust '<SERVER_URL>'` first and compare
the displayed fingerprint with the one the organizer provides.

Choose whichever programming language or languages you prefer, subject to the
contest rules and judge environment. If you plan to use `arena build` for a
compiled language, make sure Docker is available and run `arena pull` before
the contest to download the judge's build image. You can do this before signing
up. Python scripts with a `#!/usr/bin/env python3` shebang need no build image.

## Download the round materials

Run `arena round` to find the active problems. For each problem, replace `SLUG`
with its short name and run `arena problem SLUG --samples "samples/SLUG"` from
your entry directory. This displays the statement and downloads the public
sample inputs and expected outputs into a folder for that problem.

Develop and test your own solutions, submit them through the participant
interface, and use judge feedback to guide your work. Leave the judge, hidden
tests, and other contestants’ work alone.

**Keep iterating until the contest ends**, even if every solution has passed or
you lead both leaderboards. Treat each accepted solution as a baseline. Look for
a faster algorithm, a leaner data structure, or an implementation that does less
work. Test each promising change, submit it, and compare the judge’s CPU time
and memory against your previous results. If one approach stalls, try another or
revisit a different problem. Keep your best versions and allow enough time for
final submissions before the deadline.

When the contest ends, report your participant name, the problems you solved,
and your best submission IDs with their recorded CPU time and memory. Keep brief
notes on any friction you encounter so you can give specific feedback when asked
after the round.

# Post-contest follow-up

Based on your experience competing, what should we improve in Runtime Arena or
in how this event was run?

Give at least three changes in priority order. For each, describe what happened,
how it affected your participation, and what specific change would have helped.
Say whether the issue came from Arena itself, the event setup, or the shared
laptop if you can tell. Also mention one thing that worked well and should be
kept.
