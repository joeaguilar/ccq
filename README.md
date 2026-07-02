# ccq — Claude Code Query

Read-only CLI for querying Claude Code's on-disk session transcripts
(`~/.claude/projects/*/`) and agent/workflow artifacts. One tool answers:
*what commands ran, what files were written, what errored, what did the user
actually ask, which skills fired, what did subagents return, and how much of
all of the above* — without hand-rolled `jq`/`python` pipelines.

Part of the single-binary fleet convention alongside
[`itr`](https://github.com/joeaguilar/itr) (issue tracker) and
[`kgr`](https://github.com/joeaguilar/kgr) (dependency graph).

## Guarantees

- **Read-only.** `ccq` never writes inside `~/.claude/`. No lock files, no cache.
- **Streaming.** Memory is O(longest line), never O(file). 340 MB projects
  scan in well under a second at <100 MB resident.
- **Schema-tolerant.** Unknown line types are skipped, unknown fields ignored,
  malformed JSON lines counted and reported to stderr — never a crash.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/joeaguilar/ccq/main/install.sh | bash
```

Windows (PowerShell):

```powershell
iwr -useb https://raw.githubusercontent.com/joeaguilar/ccq/main/install.ps1 | iex
```

Update an existing install:

```sh
curl -fsSL https://raw.githubusercontent.com/joeaguilar/ccq/main/install.sh | bash -s -- --update
```

From source: `cargo install --path .` (or `./install.sh` inside a clone).

## Subcommands

| command | answers |
|---|---|
| `ccq projects` | which projects have transcripts, how big, how recent |
| `ccq sessions` | sessions in scope with timing and message/tool counts |
| `ccq bash` | every Bash command run (`--complex`, `--grep`) |
| `ccq writes` | every Write/Edit/NotebookEdit (`--scratch`, `--grep`) |
| `ccq tools` | tool-use frequency rollup (`--errors-only`) |
| `ccq prompts` | genuine human prompts only (wrappers/reminders excluded) |
| `ccq errors` | failed tool results with normalized recurrence signatures |
| `ccq slash` | skill/slash-command invocations |
| `ccq agents [dir]` | subagent/workflow forensics: who ran, how long, what returned |
| `ccq grep <re>` | full-text search with `--in text\|tool-use\|tool-result\|thinking` |
| `ccq show <sess>` | pretty-print one event, a window, or a whole turn |
| `ccq stats` | one-screen corpus rollup |

## Shared flags

Every subcommand takes scope filters and output controls:

```
-p, --project <pat>    substring match on project dir ('.' = current dir's project)
-s, --session <prefix> session UUID prefix (>= 4 chars)
    --since / --until  ISO date/datetime or relative (7d, 24h, 90m)
    --sidechain <include|exclude|only>   subagent traffic handling
    --root <dir>       transcript root (default: $CCQ_ROOT or ~/.claude/projects)

-f <table|tsv|json|jsonl>   output format (default: aligned table)
    --fields a,b,c     column selection
    --full             lift the 300-char truncation
    --limit <n>        cap rows (table defaults to 200)
    --count [--by <f>] aggregate into counts instead of listing
```

Exit codes: `0` results found, `1` no matches, `2` usage/IO error — so
`ccq errors -p foo --since 1d && notify` works as a gate.

## Examples

```sh
# What complex one-off scripts am I writing lately?
ccq bash --complex --since 30d -f tsv > /tmp/complex.tsv

# Most-run bash commands in a project
ccq bash -p myproj --count --by command | head -20

# Permission-denial signatures across all projects
ccq errors --count --by signature | grep -i denied

# Did any session write scratch scripts this week?
ccq writes --scratch --since 7d

# What did the user actually ask for in that session?
ccq prompts -s 8717598f

# Which skills actually get used?
ccq slash --since 90d

# Why did that workflow return empty?
ccq agents ~/…/scratchpad/wf_81a68fd2

# Find every session that touched filters.json
ccq grep 'filters\.json' --in tool-use -f tsv

# Corpus totals
ccq stats --since 2026-01-01
```

## Development

```sh
just test        # unit + integration suite
just lint        # clippy -D warnings
cargo build --release
./tests/integration.sh
```

Releases are automated: Conventional Commits on `main` drive
auto-tagging (`feat:` → minor, `fix:` → patch, `!`/`BREAKING CHANGE` → major),
and tags trigger a multi-platform release build. See `CHANGELOG.md`.

## License

MIT
