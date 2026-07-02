---
name: ccq
description: Use the `ccq` CLI (Claude Code Query) to answer questions about past Claude Code sessions instead of hand-rolling find/jq/python pipelines over ~/.claude/projects. Trigger ANY time the user asks about session history, transcript mining, or agent forensics — even if they don't say "ccq". Phrases that should fire this skill include "what commands did we run", "what did I ask in that session", "which skills get used", "what errored recently", "what did that subagent return", "why did that workflow come back empty", "find the session that touched <file>", "how big is my transcript corpus", "mine the transcripts", "reflect on past sessions", and similar. Also fire when YOU need transcript evidence (recurring failures, permission denials, scratch-script patterns) — prefer ccq over grep/jq on ~/.claude. Do NOT fire for querying the current live conversation, or for the issue tracker (itr) or code graph (kgr).
---

# ccq — query Claude Code transcripts

`ccq` is a read-only CLI over `~/.claude/projects/*/` session transcripts (already on PATH). It never writes inside `~/.claude/`, streams arbitrarily large files, and tolerates schema drift (malformed lines are counted on stderr, never fatal).

**Before the first query in a session,** run `ccq --help` once if unsure — every subcommand also has `--help` with examples. Flags below may lag the binary; trust `--help`.

## Scope and output (every subcommand)

```
-p <pat>        project dir substring; '.' = the project for the cwd; repeatable
-s <prefix>     session UUID prefix (>= 4 chars); repeatable
--since/--until ISO date/datetime or relative: 7d, 24h, 90m
--sidechain include|exclude|only   subagent traffic (default include)
-f table|tsv|json|jsonl   use tsv/json for machine reading (no 200-row cap, no truncation)
--fields a,b,c  --full  --limit N  --count [--by <field>]
```

Exit codes: `0` results, `1` no matches, `2` usage/IO error — safe to use as a gate (`ccq errors --since 1d && …`).

## Which subcommand answers what

| question | command |
|---|---|
| what projects/sessions exist, how big | `ccq projects`, `ccq sessions -p <proj>` |
| what shell commands ran | `ccq bash [-p X] [--grep RE] [--complex]` |
| what files were written/edited | `ccq writes [--scratch] [--grep RE]` |
| which tools are used most / fail | `ccq tools [--errors-only]` |
| what did the human actually ask | `ccq prompts [-s SESS]` |
| what errored, what keeps erroring | `ccq errors --count --by signature` |
| which skills/slash commands fired | `ccq slash` (defaults to count-by-command) |
| what did subagents/workflows do | `ccq agents [<run-dir>]` |
| which session touched X | `ccq grep '<regex>' --in tool-use -f tsv` |
| show me that exchange | `ccq show <sess-prefix> [--turn N \| --line N --around K]` |
| corpus totals | `ccq stats [--since T]` |

## Recipes

```sh
ccq bash --complex --since 30d -f tsv        # one-off scripting patterns (reflection mining)
ccq errors --count --by signature | grep -i denied   # permission-denial hunting
ccq prompts -s 8717598f -f json              # ground truth on what the user asked
ccq agents /path/to/scratchpad/wf_xxxx       # why did that workflow return empty?
ccq grep 'filters\.json' --in tool-use       # which sessions touched this file
ccq slash --since 90d                        # which skills actually get used
```

## Conventions

- **Prefer ccq over raw pipelines.** If you're about to `find ~/.claude/projects -name '*.jsonl' | xargs jq …`, use ccq instead; only fall back to jq for fields ccq doesn't surface.
- **Machine formats for analysis.** Use `-f tsv` or `-f jsonl` when you'll post-process; `table` truncates at 300 chars and caps at 200 rows.
- **Scope early.** `-p <proj>` and `--since` keep scans fast; full-corpus scans are still sub-second warm, but scoped output is easier to read.
- **`ccq agents` with a directory** reads exactly that run (workflow scratch dirs, `tasks/*.output`); with no arg it discovers artifacts under the transcript root.
- **Sidechain traffic is included by default.** Use `--sidechain only` to isolate subagent behavior, `exclude` for the human-facing thread.
- Don't use full paths like `~/.cargo/bin/ccq`. It's on `$PATH`.

## Updating

`ccq` self-reports its version (`ccq --version`, git-tag derived). To update:

```sh
curl -fsSL https://raw.githubusercontent.com/joeaguilar/ccq/main/install.sh | bash -s -- --update
```
