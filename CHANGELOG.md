# Changelog

## Versioning

- Release tags use `vMAJOR.MINOR.PATCH`.
- Pushes to `main` are auto-tagged by `.github/workflows/auto-version.yml`.
- A conventional commit subject with `!` after the type or scope creates a major
  bump. A `feat:` subject creates a minor bump. A `fix:` subject creates a patch
  bump. Commits without those subjects do not create a tag.
- Add `[skip version]` to the commit message to skip auto-tagging.
- Before tagging, the workflow syncs `Cargo.toml` and `Cargo.lock` to the new
  version in a `chore(release): … [skip version]` commit, and the tag points at
  that commit.
- `.github/workflows/release.yml` builds release archives and SHA256 files from
  existing `v*` tags into a draft release, and publishes it only once all 14
  assets are uploaded, so `/releases/latest` never points at a release that is
  missing downloads. GitHub release notes are generated automatically; this file
  is the terse maintained history.
- Built binaries embed `git describe --tags --always --dirty` through `build.rs`,
  falling back to the synced Cargo package version when git metadata is
  unavailable.

## v0.2.1

- `install.ps1` works on Windows PowerShell 5.1: the BOM-less script decoded as
  CP1252 there, turning em-dashes into curly quotes that broke string parsing and
  aborted the install; latest-tag resolution now passes `-UseBasicParsing`.
- `install.ps1` no longer warns that the install directory is not on PATH when it
  already is. It decides from the persistent User and Machine PATH (matching
  case-insensitively), so a directory already on the system PATH is not
  duplicated into the user scope.
- Downloads on Windows PowerShell 5.1 are no longer throttled by the progress
  bar; the caller's `$ProgressPreference` is restored afterwards.
- CI: installer smoke test on Windows PowerShell 5.1 and 7; draft-first release
  publishing; manifest sync on every auto-version tag.

## v0.1.0

Initial release, implementing the full v1 spec:

- Subcommands: `projects`, `sessions`, `bash`, `writes`, `tools`, `prompts`,
  `errors`, `slash`, `agents`, `grep`, `show`, `stats`.
- Shared scope flags (`-p`, `-s`, `--since`/`--until`, `--sidechain`, `--root`)
  and output flags (`-f table|tsv|json|jsonl`, `--fields`, `--full`, `--limit`,
  `--count --by`).
- Streaming JSONL parser: O(longest line) memory, rayon per-file parallelism,
  schema-tolerant (unknown types skipped, malformed lines counted on stderr).
- Exit codes: 0 = results, 1 = no matches, 2 = usage/IO error.
- Install scripts (`install.sh`, `install.ps1`) with prebuilt-release download,
  checksum verification, and cargo source-build fallback.
