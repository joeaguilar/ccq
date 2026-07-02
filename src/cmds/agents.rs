//! `ccq agents` — subagent/workflow forensics over `agent-*.jsonl`,
//! `tasks/*.output`, and workflow `journal.jsonl` artifacts.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::cmds::{head_of, human_duration};
use crate::out::Table;
use crate::scan::{discover_projects, for_each_line, note_malformed, parse_ts, Scope};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ArtifactKind {
    AgentTranscript,
    Journal,
}

#[allow(clippy::case_sensitive_file_extension_comparisons)] // artifact names are fixed lowercase
fn classify(path: &Path) -> Option<ArtifactKind> {
    let name = path.file_name()?.to_str()?;
    if name == "journal.jsonl" {
        return Some(ArtifactKind::Journal);
    }
    if name.starts_with("agent-") && name.ends_with(".jsonl") {
        return Some(ArtifactKind::AgentTranscript);
    }
    if name.ends_with(".output")
        && path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            == Some("tasks")
    {
        return Some(ArtifactKind::AgentTranscript);
    }
    None
}

/// Recursively collect agent artifacts under `dir`, skipping `memory/`.
fn collect_artifacts(dir: &Path, depth: usize, out: &mut Vec<(ArtifactKind, PathBuf)>) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            if name == "memory" {
                continue;
            }
            collect_artifacts(&path, depth + 1, out);
        } else if let Some(kind) = classify(&path) {
            out.push((kind, path));
        }
    }
}

/// `ccq agents [<dir>]`
pub fn agents(scope: &Scope, dir: Option<&Path>) -> Table {
    let mut artifacts: Vec<(ArtifactKind, PathBuf)> = Vec::new();
    if let Some(d) = dir {
        if d.is_file() {
            if let Some(kind) = classify(d) {
                artifacts.push((kind, d.to_path_buf()));
            } else {
                // Explicit file argument: parse it as a transcript anyway.
                artifacts.push((ArtifactKind::AgentTranscript, d.to_path_buf()));
            }
        } else {
            collect_artifacts(d, 0, &mut artifacts);
        }
    } else {
        for (_, project_dir) in discover_projects(scope) {
            collect_artifacts(&project_dir, 0, &mut artifacts);
        }
    }
    artifacts.sort();

    let mut rows: Vec<Vec<String>> = Vec::new();
    for (kind, path) in &artifacts {
        match kind {
            ArtifactKind::AgentTranscript => {
                if let Some(row) = transcript_row(path) {
                    rows.push(row);
                }
            }
            ArtifactKind::Journal => rows.extend(journal_rows(path)),
        }
    }
    Table {
        headers: vec![
            "agent",
            "started",
            "ended",
            "duration",
            "tool_uses",
            "tokens",
            "status",
            "final",
        ],
        rows,
    }
}

fn transcript_row(path: &Path) -> Option<Vec<String>> {
    let agent = path.file_stem()?.to_string_lossy().into_owned();
    let mut first_ts: Option<String> = None;
    let mut last_ts: Option<String> = None;
    let mut tool_uses = 0usize;
    let mut tokens = 0u64;
    let mut final_text = String::new();
    let malformed = for_each_line(path, |_, line| {
        if let Some(ts) = line.timestamp.as_deref() {
            if first_ts.is_none() {
                first_ts = Some(ts.to_string());
            }
            last_ts = Some(ts.to_string());
        }
        if line.ty.as_deref() == Some("assistant") {
            if let Some(usage) = line.message.as_ref().and_then(|m| m.usage) {
                tokens += usage.output_tokens.unwrap_or(0);
            }
            for b in line.blocks() {
                match b.ty.as_deref() {
                    Some("tool_use") => {
                        tool_uses += 1;
                        // A StructuredOutput call is the agent's return value.
                        if b.name.as_deref() == Some("StructuredOutput") {
                            if let Some(input) = &b.input {
                                final_text = input.to_string();
                            }
                        }
                    }
                    Some("text") => {
                        if let Some(t) = &b.text {
                            if !t.trim().is_empty() {
                                final_text = t.trim().to_string();
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    })
    .ok()?;
    note_malformed(malformed);

    let duration = match (
        first_ts.as_deref().and_then(parse_ts),
        last_ts.as_deref().and_then(parse_ts),
    ) {
        (Some(s), Some(e)) => human_duration((e - s).num_seconds()),
        _ => String::new(),
    };
    let status = if final_text.is_empty() {
        "empty"
    } else {
        "done"
    };
    Some(vec![
        agent,
        first_ts.map(|t| fmt(&t)).unwrap_or_default(),
        last_ts.map(|t| fmt(&t)).unwrap_or_default(),
        duration,
        tool_uses.to_string(),
        if tokens > 0 {
            tokens.to_string()
        } else {
            String::new()
        },
        status.to_string(),
        head_of(&final_text, 200),
    ])
}

fn fmt(ts: &str) -> String {
    parse_ts(ts).map_or_else(
        || ts.to_string(),
        |dt| dt.format("%Y-%m-%d %H:%M:%S").to_string(),
    )
}

/// Workflow journals record each `agent()` call's return value. The exact
/// shape is version-dependent; pull out whatever identifying and result
/// fields are present and skip lines that have neither.
fn journal_rows(path: &Path) -> Vec<Vec<String>> {
    let Ok(content) = fs::read_to_string(path) else {
        return vec![];
    };
    let parent = path
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut rows = Vec::new();
    for (i, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(trimmed) else {
            note_malformed(1);
            continue;
        };
        let ident = ["label", "agentId", "agent", "id", "prompt"]
            .iter()
            .find_map(|k| v.get(k).and_then(Value::as_str))
            .map_or_else(
                || format!("{parent}/journal#{}", i + 1),
                std::string::ToString::to_string,
            );
        let result = ["result", "value", "output", "return"]
            .iter()
            .find_map(|k| v.get(k))
            .map(|r| match r {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            });
        let Some(result) = result else {
            continue;
        };
        let ts = v
            .get("timestamp")
            .and_then(Value::as_str)
            .map(fmt)
            .unwrap_or_default();
        rows.push(vec![
            head_of(&ident, 60),
            ts.clone(),
            ts,
            String::new(),
            String::new(),
            String::new(),
            "journal".to_string(),
            head_of(&result, 200),
        ]);
    }
    rows
}
