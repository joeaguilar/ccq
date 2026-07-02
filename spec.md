# ccq — Claude Code Query

**Status:** spec, v1 — ready to build
**Shape:** single Rust CLI binary (fleet convention: `itr`, `kgr`, `plr`), read-only, zero-config
**Origin:** `claude-reflection-notes.md` finding #2 (2026-07-02). The `find ~/.claude/projects -name '*.jsonl' | xargs jq 'select(.type=="assistant") | .message.content[]? | select(.type=="tool_use") …'` pipeline has been hand-derived in at least five sessions (werkit:8717598f ×22 aggregations, werkit:debe74b6, Harness:8796eb93 ×6, Harness:6c8ce498 ×4), each burning subagent time against a ~520 MB JSONL corpus. `ccq` makes every one of those questions a one-liner.

---

## 1. Purpose

Query Claude Code's on-disk session transcripts (`~/.claude/projects/*/`) and agent/workflow artifacts without hand-rolled jq/python. One tool answers: *what commands ran, what files were written, what errored, what did the user actually ask, which skills fired, what did subagents return, and how much of all of the above.*

**Hard guarantees:**
- **Read-only.** `ccq` never writes inside `~/.claude/`. No lock files, no cache in v1.
- **Streaming.** Memory is O(longest line), never O(file). Single sessions reach 100 MB+; one project is 341 MB.
- **Schema-tolerant.** Transcript format evolves between Claude Code versions. Unknown line `type`s are skipped, unknown fields ignored, malformed JSON lines counted and reported to stderr (`ccq: skipped 3 malformed lines`) — never a crash.

## 2. Data model (as observed 2026-07)

### 2.1 Location & project encoding
- Transcript root: `~/.claude/projects/` (overridable: `--root`, `$CCQ_ROOT`).
- One directory per project cwd, encoded by replacing `/` with `-`: `/Users/x/AI_Projects/werkit` → `-Users-x-AI_Projects-werkit`. **The encoding is lossy** (hyphens in real paths collide with separators): ccq must treat dir names as opaque IDs and match them by substring/suffix, never round-trip them back to paths. When a decoded guess is needed (display), best-effort decode with a `~`-relative render.
- Sessions: `<uuid>.jsonl` inside the project dir. Session ID = file stem.
- Adjacent artifacts (some inside session-scratch dirs, some under project transcript dirs): workflow `journal.jsonl`, `agent-<id>.jsonl`, `tasks/<taskid>.output` (a full subagent JSONL transcript). `memory/` subdirs are *not* transcripts — exclude from scans.

### 2.2 Line shapes (observed)
Every line is one JSON object with a `type` discriminator. Observed types and the fields ccq relies on:

| type | relevant fields |
|---|---|
| `user` | `message.role`, `message.content` (string **or** array of blocks incl. `tool_result`), `sessionId`, `timestamp`, `cwd`, `isSidechain`, `parentUuid` |
| `assistant` | `message.content[]` blocks: `text`, `tool_use` (`name`, `input`), `thinking`; plus same envelope fields as `user` |
| `mode` | `mode`, `sessionId` |
| `file-history-snapshot` | ignored |
| `summary` / others | skipped, counted under `--verbose` |

`tool_result` blocks appear in `user`-typed lines (`content[].type == "tool_result"`, with `is_error`, `content`). Sidechain lines (`isSidechain: true`) are subagent traffic embedded in the parent session file.

**Parsing rule:** `message.content` must accept both `String` and `Vec<Block>` (both occur). Every extractor works off this normalized event stream.

## 3. CLI surface

```
ccq <subcommand> [scope] [filters] [output]
```

### 3.1 Shared scope & filter flags (every subcommand)
| flag | meaning |
|---|---|
| `-p, --project <pat>` | substring match on project dir name (`-p werkit` → `…-AI_Projects-werkit`). Repeatable. `-p .` = project matching the current working directory. Default: **all projects** |
| `-s, --session <prefix>` | session UUID prefix (≥4 chars). Repeatable |
| `--since <t>` / `--until <t>` | ISO date/datetime or relative (`7d`, `24h`, `90m`) against line `timestamp` |
| `--sidechain <include\|exclude\|only>` | default `include`; `only` isolates subagent traffic |
| `--limit <n>` | cap output rows (default: unlimited for `-f` machine formats, 200 for human tables) |

### 3.2 Shared output flags
| flag | meaning |
|---|---|
| `-f <table\|tsv\|json\|jsonl>` | default `table` (aligned, human). `tsv` escapes tabs/newlines (`\t`→space, `\n`→`⏎`). `json` = one array; `jsonl` = one object per row |
| `--fields <a,b,c>` | column selection; each subcommand documents its field set |
| `--full` | disable the default 300-char truncation of long text fields |
| `--count [--by <field>]` | aggregate instead of listing: row counts grouped by a field |

Exit codes: `0` results found, `1` no matches, `2` usage/IO error. (Enables `ccq errors -p foo --since 1d && …` gating.)

### 3.3 Subcommands

#### `ccq projects`
List projects with session count, total size, first/last activity.
Fields: `project, sessions, size, first, last`.

#### `ccq sessions`
List sessions in scope: `session, project, start, end, duration, user_msgs, asst_msgs, tools, cwd`.

#### `ccq bash`
Every Bash `tool_use`: `ts, session, project, description, command`.
Extra filters: `--grep <re>` (on command), `--complex` (heuristic: multi-line, heredoc, `python3 -c`/`node -e`, `jq`/`awk`/`sed` program, `for`/`while`).
> Replaces the exact jq pipeline this spec was born from.

#### `ccq writes`
`Write`/`Edit`/`NotebookEdit` tool_use: `ts, session, tool, file_path, head` (first 200 chars of content/new_string).
Extra: `--scratch` (only paths under `/tmp`, `/private/tmp`, `*scratchpad*`), `--grep <re>` (on path).

#### `ccq tools`
Tool-use frequency table: `tool, count, projects, sessions` — the corpus rollup, default `--count --by tool`. `--errors-only` cross-joins with failed results.

#### `ccq prompts`
Genuine human prompts only: from `user` lines with string/text content, **excluding** `tool_result` carriers, `<local-command-caveat>`/`<command-name>` wrappers, and `<system-reminder>`-only bodies. Fields: `ts, session, project, prompt`.

#### `ccq errors`
`tool_result` blocks with `is_error: true`: `ts, session, tool (resolved via the paired tool_use id), signature, snippet`. `signature` = first line, normalized (paths/numbers stripped) so `--count --by signature` surfaces recurring failures — permission denials, missing binaries, Edit-anchor misses. This is the `/fewer-permission-prompts` scan, generalized.

#### `ccq slash`
Skill/slash invocations from `<command-name>` markers and `Skill` tool_use: `ts, session, command, args`. Default `--count --by command`.

#### `ccq agents [<dir>]`
Subagent/workflow forensics. With no arg, discovers `tasks/*.output`, `agent-*.jsonl`, `journal.jsonl` in scope; with a dir, reads exactly that run. Reports per agent: `agent, started, ended, duration, tool_uses, tokens (when present), status, final` (final text/StructuredOutput, truncated). Answers "spawned vs completed, who failed, what did each return" — currently re-scripted every time (Harness:8796eb93 ×5, good:11901612 ×9).

#### `ccq grep <regex>`
Full-text search with typed scoping: `--in <text|tool_use|tool_result|thinking|any>` (default `any`). Returns `session, line_no, type, match_context`. Pointer-oriented — pair with `ccq show`.

#### `ccq show <session-prefix> [--line N] [--around K] [--turn N]`
Pretty-print one event, a window, or a whole turn (user msg + assistant response + tool round-trips) with roles, tool names, and truncated payloads. The "let me actually look at that exchange" tool.

#### `ccq stats`
One-screen corpus rollup: totals from `projects`/`sessions`/`tools` plus bash-complexity ratio and writes-to-scratch count. This is the report the five mining subagents each rebuilt by hand.

## 4. Worked examples (each replaces a mined incantation)

```sh
# What complex one-off scripts am I writing lately? (the reflection question)
ccq bash --complex --since 30d -f tsv > /tmp/complex.tsv

# Most-run bash commands in a project
ccq bash -p rustglichur --count --by command | head -20

# Permission-denial signatures across all projects (fewer-permission-prompts)
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
ccq grep 'filters\.json' --in tool_use -f tsv

# Reproduce this reflection's TOTALS instantly
ccq stats --since 2026-01-01
```

## 5. Implementation notes

- **Language/deps:** Rust; `clap` (derive), `serde`/`serde_json` (`RawValue` + manual envelope structs; do not fully deserialize unknown blocks), `rayon` for per-file parallelism, `jiff` or `chrono` for time. No async needed.
- **Performance target:** full-corpus scan (~520 MB) < 5 s warm on this machine; single project < 1 s. Parallelize per file; within a file, stream line-by-line (`BufReader`, 1 MB line buffer, tolerate longer).
- **Truncation:** all human output truncates long fields at 300 chars with `…`; `--full` lifts. Machine formats never truncate unless `--limit`.
- **Robustness tests (fixtures, committed):** string-vs-array `message.content`; malformed line mid-file; unknown `type`; 5 MB single line; empty session; sidechain-only session; `tasks/*.output` parsing.
- **Non-goals (v1):** no index/cache, no watch mode, no redaction, no writing anywhere, no cost/token accounting beyond what lines already carry, no cross-machine sync. Revisit an index only if corpus growth makes scans exceed ~10 s.
- **Install:** `cargo install --path .`, plus `install.sh` matching itr/kgr.

## 6. Acceptance (definition of done for v1)

1. Every §4 example runs against the real corpus and returns plausible, spot-checked results.
2. `ccq bash -p werkit` output matches a hand-run jq extraction on one session file (golden test).
3. Full-corpus `ccq stats` completes < 5 s and skips zero-byte/malformed input without error exit.
4. `ccq` run with no args prints subcommand help; every subcommand has `--help` with at least one example.
5. Memory stays < 200 MB on the 341 MB project (verified once with `/usr/bin/time -l`).
