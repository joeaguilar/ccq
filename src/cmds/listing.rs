//! Row-producing subcommands: projects, sessions, bash, writes, tools,
//! prompts, errors, slash.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};

use regex::Regex;
use serde_json::Value;

use crate::cmds::{
    error_signature, extract_prompt, head_of, human_duration, is_complex_command, is_scratch_path,
};
use crate::model::{tool_result_text, RawLine};
use crate::out::{human_size, Table};
use crate::scan::{
    discover_sessions, for_each_line, map_sessions, note_malformed, parse_ts, Scope, SessionFile,
};

fn fmt_ts(raw: &str) -> String {
    parse_ts(raw).map_or_else(
        || raw.to_string(),
        |dt| dt.format("%Y-%m-%d %H:%M:%S").to_string(),
    )
}

fn line_ts(line: &RawLine) -> String {
    line.timestamp.as_deref().map(fmt_ts).unwrap_or_default()
}

fn short_session(id: &str) -> String {
    id.chars().take(8).collect()
}

/// `ccq projects` — cheap scan: first-line timestamp + file mtime, no full parse.
pub fn projects(scope: &Scope) -> Table {
    let files = discover_sessions(scope);
    let mut agg: HashMap<String, (usize, u64, Option<String>, Option<String>)> = HashMap::new();
    for sf in &files {
        let first = first_line_timestamp(sf);
        let last = std::fs::metadata(&sf.path)
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|t| {
                chrono::DateTime::<chrono::Utc>::from(t)
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string()
            });
        let entry = agg.entry(sf.project.clone()).or_insert((0, 0, None, None));
        entry.0 += 1;
        entry.1 += sf.size;
        if let Some(f) = first {
            if entry.2.as_deref().is_none_or(|cur| f.as_str() < cur) {
                entry.2 = Some(f);
            }
        }
        if let Some(l) = last {
            if entry.3.as_deref().is_none_or(|cur| l.as_str() > cur) {
                entry.3 = Some(l);
            }
        }
    }
    let mut rows: Vec<Vec<String>> = agg
        .into_iter()
        .map(|(project, (sessions, size, first, last))| {
            vec![
                project,
                sessions.to_string(),
                human_size(size),
                first.unwrap_or_default(),
                last.unwrap_or_default(),
            ]
        })
        .collect();
    rows.sort_by(|a, b| b[4].cmp(&a[4]));
    Table {
        headers: vec!["project", "sessions", "size", "first", "last"],
        rows,
    }
}

fn first_line_timestamp(sf: &SessionFile) -> Option<String> {
    let file = std::fs::File::open(&sf.path).ok()?;
    let mut reader = BufReader::with_capacity(64 * 1024, file);
    let mut buf = String::new();
    // First few lines: the very first can be a summary/snapshot without ts.
    for _ in 0..5 {
        buf.clear();
        if reader.read_line(&mut buf).ok()? == 0 {
            return None;
        }
        if let Ok(raw) = serde_json::from_str::<RawLine>(buf.trim()) {
            if let Some(ts) = raw.timestamp.as_deref() {
                return Some(fmt_ts(ts));
            }
        }
    }
    None
}

/// `ccq sessions`
pub fn sessions(scope: &Scope) -> Table {
    let files = discover_sessions(scope);
    let rows = map_sessions(&files, |sf| {
        let mut start: Option<String> = None;
        let mut end: Option<String> = None;
        let mut user_msgs = 0usize;
        let mut asst_msgs = 0usize;
        let mut tools = 0usize;
        let mut cwd = String::new();
        let mut any = false;
        let malformed = for_each_line(&sf.path, |_, line| {
            if !scope.line_in_scope(&line) {
                return;
            }
            any = true;
            if let Some(ts) = line.timestamp.as_deref() {
                if start.as_deref().is_none_or(|s| ts < s) {
                    start = Some(ts.to_string());
                }
                if end.as_deref().is_none_or(|e| ts > e) {
                    end = Some(ts.to_string());
                }
            }
            if cwd.is_empty() {
                if let Some(c) = line.cwd.as_deref() {
                    cwd = c.to_string();
                }
            }
            match line.ty.as_deref() {
                Some("assistant") => {
                    asst_msgs += 1;
                    tools += line
                        .blocks()
                        .iter()
                        .filter(|b| b.ty.as_deref() == Some("tool_use"))
                        .count();
                }
                Some("user") if extract_prompt(&line).is_some() => {
                    user_msgs += 1;
                }
                _ => {}
            }
        })?;
        note_malformed(malformed);
        if !any {
            return Ok(vec![]);
        }
        let duration = match (start.as_deref(), end.as_deref()) {
            (Some(s), Some(e)) => match (parse_ts(s), parse_ts(e)) {
                (Some(s), Some(e)) => human_duration((e - s).num_seconds()),
                _ => String::new(),
            },
            _ => String::new(),
        };
        Ok(vec![vec![
            sf.session_id.clone(),
            sf.project.clone(),
            start.as_deref().map(fmt_ts).unwrap_or_default(),
            end.as_deref().map(fmt_ts).unwrap_or_default(),
            duration,
            user_msgs.to_string(),
            asst_msgs.to_string(),
            tools.to_string(),
            cwd,
        ]])
    });
    let mut rows = rows;
    rows.sort_by(|a, b| b[2].cmp(&a[2]));
    Table {
        headers: vec![
            "session",
            "project",
            "start",
            "end",
            "duration",
            "user_msgs",
            "asst_msgs",
            "tools",
            "cwd",
        ],
        rows,
    }
}

/// `ccq bash`
pub fn bash(scope: &Scope, grep: Option<&Regex>, complex: bool) -> Table {
    let files = discover_sessions(scope);
    let rows = map_sessions(&files, |sf| {
        let mut rows = Vec::new();
        let malformed = for_each_line(&sf.path, |_, line| {
            if line.ty.as_deref() != Some("assistant") || !scope.line_in_scope(&line) {
                return;
            }
            for b in line.blocks() {
                if b.ty.as_deref() != Some("tool_use") || b.name.as_deref() != Some("Bash") {
                    continue;
                }
                let input = b.input.as_ref();
                let command = input
                    .and_then(|i| i.get("command"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if command.is_empty() {
                    continue;
                }
                if complex && !is_complex_command(command) {
                    continue;
                }
                if let Some(re) = grep {
                    if !re.is_match(command) {
                        continue;
                    }
                }
                let description = input
                    .and_then(|i| i.get("description"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                rows.push(vec![
                    line_ts(&line),
                    short_session(&sf.session_id),
                    sf.project.clone(),
                    description.to_string(),
                    command.to_string(),
                ]);
            }
        })?;
        note_malformed(malformed);
        Ok(rows)
    });
    Table {
        headers: vec!["ts", "session", "project", "description", "command"],
        rows,
    }
}

/// `ccq writes`
pub fn writes(scope: &Scope, scratch: bool, grep: Option<&Regex>) -> Table {
    let files = discover_sessions(scope);
    let rows = map_sessions(&files, |sf| {
        let mut rows = Vec::new();
        let malformed = for_each_line(&sf.path, |_, line| {
            if line.ty.as_deref() != Some("assistant") || !scope.line_in_scope(&line) {
                return;
            }
            for b in line.blocks() {
                if b.ty.as_deref() != Some("tool_use") {
                    continue;
                }
                let tool = b.name.as_deref().unwrap_or("");
                if !matches!(tool, "Write" | "Edit" | "NotebookEdit") {
                    continue;
                }
                let input = b.input.as_ref();
                let path = input
                    .and_then(|i| i.get("file_path").or_else(|| i.get("notebook_path")))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if scratch && !is_scratch_path(path) {
                    continue;
                }
                if let Some(re) = grep {
                    if !re.is_match(path) {
                        continue;
                    }
                }
                let body = input
                    .and_then(|i| {
                        i.get("content")
                            .or_else(|| i.get("new_string"))
                            .or_else(|| i.get("new_source"))
                    })
                    .and_then(Value::as_str)
                    .unwrap_or("");
                rows.push(vec![
                    line_ts(&line),
                    short_session(&sf.session_id),
                    sf.project.clone(),
                    tool.to_string(),
                    path.to_string(),
                    head_of(body, 200),
                ]);
            }
        })?;
        note_malformed(malformed);
        Ok(rows)
    });
    Table {
        headers: vec!["ts", "session", "project", "tool", "file_path", "head"],
        rows,
    }
}

/// `ccq tools` — corpus rollup: tool, count, projects, sessions.
pub fn tools(scope: &Scope, errors_only: bool) -> Table {
    let files = discover_sessions(scope);
    // Per file: (tool, project, session) triples for each qualifying use.
    let triples = map_sessions(&files, |sf| {
        let mut uses: Vec<(String, String)> = Vec::new(); // (id, tool)
        let mut error_ids: HashSet<String> = HashSet::new();
        let malformed = for_each_line(&sf.path, |_, line| {
            if !scope.line_in_scope(&line) {
                return;
            }
            for b in line.blocks() {
                match b.ty.as_deref() {
                    Some("tool_use") => {
                        if let Some(name) = b.name.as_deref() {
                            uses.push((b.id.clone().unwrap_or_default(), name.to_string()));
                        }
                    }
                    Some("tool_result") if b.is_error == Some(true) => {
                        if let Some(id) = b.tool_use_id.as_deref() {
                            error_ids.insert(id.to_string());
                        }
                    }
                    _ => {}
                }
            }
        })?;
        note_malformed(malformed);
        Ok(uses
            .into_iter()
            .filter(|(id, _)| !errors_only || error_ids.contains(id))
            .map(|(_, tool)| (tool, sf.project.clone(), sf.session_id.clone()))
            .collect())
    });
    let mut agg: HashMap<String, (usize, HashSet<String>, HashSet<String>)> = HashMap::new();
    for (tool, project, session) in triples {
        let e = agg.entry(tool).or_default();
        e.0 += 1;
        e.1.insert(project);
        e.2.insert(session);
    }
    let mut rows: Vec<Vec<String>> = agg
        .into_iter()
        .map(|(tool, (count, projects, sessions))| {
            vec![
                tool,
                count.to_string(),
                projects.len().to_string(),
                sessions.len().to_string(),
            ]
        })
        .collect();
    rows.sort_by(|a, b| {
        b[1].parse::<usize>()
            .unwrap_or(0)
            .cmp(&a[1].parse::<usize>().unwrap_or(0))
    });
    Table {
        headers: vec!["tool", "count", "projects", "sessions"],
        rows,
    }
}

/// `ccq prompts`
pub fn prompts(scope: &Scope) -> Table {
    let files = discover_sessions(scope);
    let rows = map_sessions(&files, |sf| {
        let mut rows = Vec::new();
        let malformed = for_each_line(&sf.path, |_, line| {
            if !scope.line_in_scope(&line) {
                return;
            }
            if let Some(prompt) = extract_prompt(&line) {
                rows.push(vec![
                    line_ts(&line),
                    short_session(&sf.session_id),
                    sf.project.clone(),
                    prompt,
                ]);
            }
        })?;
        note_malformed(malformed);
        Ok(rows)
    });
    Table {
        headers: vec!["ts", "session", "project", "prompt"],
        rows,
    }
}

/// `ccq errors`
pub fn errors(scope: &Scope) -> Table {
    let files = discover_sessions(scope);
    let rows = map_sessions(&files, |sf| {
        let mut tool_by_id: HashMap<String, String> = HashMap::new();
        let mut rows = Vec::new();
        let malformed = for_each_line(&sf.path, |_, line| {
            if !scope.line_in_scope(&line) {
                return;
            }
            for b in line.blocks() {
                match b.ty.as_deref() {
                    Some("tool_use") => {
                        if let (Some(id), Some(name)) = (b.id.as_deref(), b.name.as_deref()) {
                            tool_by_id.insert(id.to_string(), name.to_string());
                        }
                    }
                    Some("tool_result") if b.is_error == Some(true) => {
                        let tool = b
                            .tool_use_id
                            .as_deref()
                            .and_then(|id| tool_by_id.get(id))
                            .cloned()
                            .unwrap_or_default();
                        let text = b.content.as_ref().map(tool_result_text).unwrap_or_default();
                        rows.push(vec![
                            line_ts(&line),
                            short_session(&sf.session_id),
                            sf.project.clone(),
                            tool,
                            error_signature(&text),
                            head_of(&text, 200),
                        ]);
                    }
                    _ => {}
                }
            }
        })?;
        note_malformed(malformed);
        Ok(rows)
    });
    Table {
        headers: vec!["ts", "session", "project", "tool", "signature", "snippet"],
        rows,
    }
}

/// `ccq slash`
pub fn slash(scope: &Scope) -> Table {
    let cmd_re = Regex::new(r"<command-name>\s*/?([^<]*?)\s*</command-name>").expect("static re");
    let args_re = Regex::new(r"(?s)<command-args>(.*?)</command-args>").expect("static re");
    let files = discover_sessions(scope);
    let rows = map_sessions(&files, |sf| {
        let mut rows = Vec::new();
        let malformed = for_each_line(&sf.path, |_, line| {
            if !scope.line_in_scope(&line) {
                return;
            }
            match line.ty.as_deref() {
                Some("user") => {
                    let text = match line.string_content() {
                        Some(s) => s.to_string(),
                        None => line
                            .blocks()
                            .iter()
                            .filter(|b| b.ty.as_deref() == Some("text"))
                            .filter_map(|b| b.text.clone())
                            .collect::<Vec<_>>()
                            .join("\n"),
                    };
                    if let Some(cap) = cmd_re.captures(&text) {
                        let name = cap[1].trim();
                        if !name.is_empty() {
                            let args = args_re
                                .captures(&text)
                                .map(|c| c[1].trim().to_string())
                                .unwrap_or_default();
                            rows.push(vec![
                                line_ts(&line),
                                short_session(&sf.session_id),
                                sf.project.clone(),
                                format!("/{name}"),
                                head_of(&args, 200),
                            ]);
                        }
                    }
                }
                Some("assistant") => {
                    for b in line.blocks() {
                        if b.ty.as_deref() == Some("tool_use") && b.name.as_deref() == Some("Skill")
                        {
                            let input = b.input.as_ref();
                            let skill = input
                                .and_then(|i| i.get("skill"))
                                .and_then(Value::as_str)
                                .unwrap_or("");
                            if skill.is_empty() {
                                continue;
                            }
                            let args = input
                                .and_then(|i| i.get("args"))
                                .and_then(Value::as_str)
                                .unwrap_or("");
                            rows.push(vec![
                                line_ts(&line),
                                short_session(&sf.session_id),
                                sf.project.clone(),
                                format!("/{skill}"),
                                head_of(args, 200),
                            ]);
                        }
                    }
                }
                _ => {}
            }
        })?;
        note_malformed(malformed);
        Ok(rows)
    });
    Table {
        headers: vec!["ts", "session", "project", "command", "args"],
        rows,
    }
}
