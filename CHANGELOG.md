# Changelog

## Versioning

- Release tags use `vMAJOR.MINOR.PATCH`.
- Pushes to `main` are auto-tagged by `.github/workflows/auto-version.yml`.
- A conventional commit subject with `!` after the type or scope creates a major
  bump. A `feat:` subject creates a minor bump. A `fix:` subject creates a patch
  bump. Commits without those subjects do not create a tag.
- Add `[skip version]` to the commit message to skip auto-tagging.
- `.github/workflows/release.yml` builds release archives and SHA256 files from
  existing `v*` tags. GitHub release notes are generated automatically; this file
  is the terse maintained history.
- Built binaries embed `git describe --tags --always --dirty` through `build.rs`,
  falling back to the Cargo package version when git metadata is unavailable.
  `Cargo.toml`'s `version` field is intentionally pinned at `0.1.0`.

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
