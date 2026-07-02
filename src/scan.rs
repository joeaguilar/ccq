//! Corpus discovery and streaming line iteration.
//!
//! Memory stays O(longest line): sessions are read line-by-line through a
//! reused `String` buffer. Files are processed in parallel with rayon; each
//! file returns its partial result plus a malformed-line count that the
//! caller reports once on stderr.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::{DateTime, Utc};
use rayon::prelude::*;

use crate::model::RawLine;

#[derive(Clone, Copy, PartialEq, Eq, Debug, clap::ValueEnum)]
pub enum Sidechain {
    Include,
    Exclude,
    Only,
}

#[derive(Clone, Debug)]
pub struct Scope {
    pub root: PathBuf,
    pub projects: Vec<String>,
    pub sessions: Vec<String>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub sidechain: Sidechain,
}

#[derive(Clone, Debug)]
pub struct SessionFile {
    pub path: PathBuf,
    pub project: String,
    pub session_id: String,
    pub size: u64,
}

/// Encode a filesystem path the way Claude Code encodes project cwds into
/// transcript directory names (`/` → `-`). Lossy by design; used only to
/// resolve `-p .`.
pub fn encode_project_path(path: &Path) -> String {
    path.to_string_lossy().replace(['/', '.'], "-")
}

fn project_matches(dir_name: &str, patterns: &[String]) -> bool {
    if patterns.is_empty() {
        return true;
    }
    patterns.iter().any(|p| {
        if p == "." {
            if let Ok(cwd) = std::env::current_dir() {
                return dir_name == encode_project_path(&cwd);
            }
            return false;
        }
        dir_name.contains(p.as_str())
    })
}

fn session_matches(stem: &str, prefixes: &[String]) -> bool {
    if prefixes.is_empty() {
        return true;
    }
    prefixes.iter().any(|p| stem.starts_with(p.as_str()))
}

/// List project directories in scope (name, path).
pub fn discover_projects(scope: &Scope) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(&scope.root) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "memory" || name.starts_with('.') {
            continue;
        }
        if project_matches(&name, &scope.projects) {
            out.push((name, path));
        }
    }
    out.sort();
    out
}

/// List session transcript files in scope.
pub fn discover_sessions(scope: &Scope) -> Vec<SessionFile> {
    let mut out = Vec::new();
    for (project, dir) in discover_projects(scope) {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() || path.extension().is_none_or(|e| e != "jsonl") {
                continue;
            }
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            if !session_matches(&stem, &scope.sessions) {
                continue;
            }
            let size = entry.metadata().map_or(0, |m| m.len());
            out.push(SessionFile {
                path,
                project: project.clone(),
                session_id: stem,
                size,
            });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Parse an RFC3339-ish transcript timestamp.
pub fn parse_ts(ts: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

impl Scope {
    /// Line-level filter shared by every subcommand: time window + sidechain.
    pub fn line_in_scope(&self, line: &RawLine) -> bool {
        let sidechain = line.is_sidechain.unwrap_or(false);
        match self.sidechain {
            Sidechain::Include => {}
            Sidechain::Exclude if sidechain => return false,
            Sidechain::Only if !sidechain => return false,
            _ => {}
        }
        if self.since.is_some() || self.until.is_some() {
            let Some(ts) = line.timestamp.as_deref().and_then(parse_ts) else {
                // Lines without timestamps can't be placed in the window;
                // drop them when a window was requested.
                return false;
            };
            if let Some(since) = self.since {
                if ts < since {
                    return false;
                }
            }
            if let Some(until) = self.until {
                if ts > until {
                    return false;
                }
            }
        }
        true
    }
}

/// Stream every parseable line of a JSONL file through `f(line_no, raw)`.
/// Returns the number of malformed (non-empty, unparseable) lines.
pub fn for_each_line(path: &Path, mut f: impl FnMut(usize, RawLine)) -> std::io::Result<usize> {
    let file = fs::File::open(path)?;
    let mut reader = BufReader::with_capacity(1 << 20, file);
    let mut buf = String::new();
    let mut line_no = 0usize;
    let mut malformed = 0usize;
    loop {
        buf.clear();
        if reader.read_line(&mut buf)? == 0 {
            break;
        }
        line_no += 1;
        let trimmed = buf.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<RawLine>(trimmed) {
            Ok(raw) => f(line_no, raw),
            Err(_) => malformed += 1,
        }
    }
    Ok(malformed)
}

static MALFORMED: AtomicUsize = AtomicUsize::new(0);

pub fn note_malformed(n: usize) {
    MALFORMED.fetch_add(n, Ordering::Relaxed);
}

/// Print the corpus-wide malformed-line count once, at exit.
pub fn report_malformed() {
    let n = MALFORMED.load(Ordering::Relaxed);
    if n > 0 {
        eprintln!(
            "ccq: skipped {n} malformed line{}",
            if n == 1 { "" } else { "s" }
        );
    }
}

/// Run `per_file` over every session in parallel and concatenate results.
/// IO errors are reported to stderr and skipped (a vanished session file
/// mid-scan is not fatal).
pub fn map_sessions<T: Send>(
    files: &[SessionFile],
    per_file: impl Fn(&SessionFile) -> std::io::Result<Vec<T>> + Sync,
) -> Vec<T> {
    let mut results: Vec<(usize, Vec<T>)> = files
        .par_iter()
        .enumerate()
        .filter_map(|(i, sf)| match per_file(sf) {
            Ok(v) => Some((i, v)),
            Err(e) => {
                eprintln!("ccq: {}: {e}", sf.path.display());
                None
            }
        })
        .collect();
    results.sort_by_key(|(i, _)| *i);
    results.into_iter().flat_map(|(_, v)| v).collect()
}
