// src/version_shape.rs
//
// Shapes the output of `git describe --tags --always --dirty` into the
// version string baked into the binary as CCQ_VERSION.
//
// This file is NOT a module: it is textually `include!`d from two places so
// the exact code the build script runs is also covered by `cargo test`:
//   - build.rs (the real consumer, at compile time)
//   - src/main.rs's `version_shape_tests` module (test-only)
// Keep it dependency-free (std only) and free of `mod`/`use` items.

/// Shape a raw `git describe --tags --always --dirty` result into a
/// semver-looking version string.
///
/// `git describe` has three relevant outcomes:
///   - a tag-based description (`v0.2.0`, `v0.2.0-4-gf40ddd4[-dirty]`):
///     returned unchanged.
///   - a bare commit hash (`f40ddd4[-dirty]`) — the `--always` fallback when
///     no tag is reachable: soft-fallback to `<pkg_version>+<hash>[-dirty]`
///     so the version stays semver-shaped.
///   - no output at all (no git, not a repo): fall back to `pkg_version`.
fn shape_version(git_describe: Option<&str>, pkg_version: &str) -> String {
    let Some(describe) = git_describe else {
        return pkg_version.to_string();
    };
    let core = describe.strip_suffix("-dirty").unwrap_or(describe);
    let is_bare_hash = (7..=40).contains(&core.len())
        && core
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
    if is_bare_hash {
        format!("{pkg_version}+{describe}")
    } else {
        describe.to_string()
    }
}
