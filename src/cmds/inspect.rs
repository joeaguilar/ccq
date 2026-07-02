//! `ccq grep` and `ccq show` — pointer search and single-session pretty print.

use regex::Regex;
use serde_json::Value;

use crate::cmds::{extract_prompt, head_of};
use crate::model::{tool_result_text, RawLine};
use crate::out::Table;
use crate::scan::{
    discover_sessions, for_each_line, map_sessions, note_malformed, Scope, SessionFile,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug, clap::ValueEnum)]
pub enum GrepIn {
    Text,
    ToolUse,
    ToolResult,
    Thinking,
    Any,
}

/// Typed text segments of one transcript line, for `grep --in`.
fn segments(line: &RawLine) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if let Some(s) = line.string_content() {
        out.push(("text", s.to_string()));
    }
    for b in line.blocks() {
        match b.ty.as_deref() {
            Some("text") => {
                if let Some(t) = &b.text {
                    out.push(("text", t.clone()));
                }
            }
            Some("thinking") => {
                if let Some(t) = &b.thinking {
                    out.push(("thinking", t.clone()));
                }
            }
            Some("tool_use") => {
                let name = b.name.as_deref().unwrap_or("?");
                let input = b
                    .input
                    .as_ref()
                    .map(std::string::ToString::to_string)
                    .unwrap_or_default();
                out.push(("tool_use", format!("{name} {input}")));
            }
            Some("tool_result") => {
                if let Some(c) = &b.content {
                    out.push(("tool_result", tool_result_text(c)));
                }
            }
            _ => {}
        }
    }
    out
}

fn segment_selected(kind: &str, want: GrepIn) -> bool {
    match want {
        GrepIn::Any => true,
        GrepIn::Text => kind == "text",
        GrepIn::ToolUse => kind == "tool_use",
        GrepIn::ToolResult => kind == "tool_result",
        GrepIn::Thinking => kind == "thinking",
    }
}

fn match_context(text: &str, re: &Regex) -> Option<String> {
    let m = re.find(text)?;
    let chars: Vec<char> = text.chars().collect();
    // Map byte offsets to char offsets for a safe window.
    let start_c = text[..m.start()].chars().count();
    let end_c = start_c + text[m.start()..m.end()].chars().count();
    let from = start_c.saturating_sub(60);
    let to = (end_c + 60).min(chars.len());
    let mut ctx: String = chars[from..to].iter().collect();
    ctx = ctx.replace(['\n', '\t'], " ");
    if from > 0 {
        ctx = format!("…{ctx}");
    }
    if to < chars.len() {
        ctx.push('…');
    }
    Some(ctx)
}

/// `ccq grep <regex>`
pub fn grep(scope: &Scope, re: &Regex, want: GrepIn) -> Table {
    let files = discover_sessions(scope);
    let rows = map_sessions(&files, |sf| {
        let mut rows = Vec::new();
        let malformed = for_each_line(&sf.path, |line_no, line| {
            if !scope.line_in_scope(&line) {
                return;
            }
            for (kind, text) in segments(&line) {
                if !segment_selected(kind, want) {
                    continue;
                }
                if let Some(ctx) = match_context(&text, re) {
                    rows.push(vec![
                        sf.session_id.clone(),
                        sf.project.clone(),
                        line_no.to_string(),
                        kind.to_string(),
                        ctx,
                    ]);
                }
            }
        })?;
        note_malformed(malformed);
        Ok(rows)
    });
    Table {
        headers: vec!["session", "project", "line_no", "type", "match_context"],
        rows,
    }
}

/// Find the unique session whose id starts with `prefix` within scope.
pub fn find_session(scope: &Scope, prefix: &str) -> Result<SessionFile, String> {
    let mut wide = scope.clone();
    wide.sessions = vec![];
    let matches: Vec<SessionFile> = discover_sessions(&wide)
        .into_iter()
        .filter(|sf| sf.session_id.starts_with(prefix))
        .collect();
    match matches.len() {
        0 => Err(format!("no session matching '{prefix}' in scope")),
        1 => Ok(matches.into_iter().next().expect("len checked")),
        n => Err(format!(
            "session prefix '{prefix}' is ambiguous ({n} matches): {}",
            matches
                .iter()
                .take(5)
                .map(|m| m.session_id.chars().take(12).collect::<String>())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// `ccq show` — pretty-print events. Returns the number of events printed.
pub fn show(
    scope: &Scope,
    sf: &SessionFile,
    line: Option<usize>,
    around: usize,
    turn: Option<usize>,
    full: bool,
) -> std::io::Result<usize> {
    // Turn boundaries need a first pass: line numbers of genuine prompts.
    let mut turn_starts: Vec<usize> = Vec::new();
    if turn.is_some() {
        let malformed = for_each_line(&sf.path, |line_no, l| {
            if extract_prompt(&l).is_some() {
                turn_starts.push(line_no);
            }
        })?;
        note_malformed(malformed);
    }
    let (win_start, win_end) = if let Some(n) = line {
        (n.saturating_sub(around), n + around)
    } else if let Some(t) = turn {
        if t == 0 || t > turn_starts.len() {
            eprintln!(
                "ccq: session has {} turns (asked for turn {t})",
                turn_starts.len()
            );
            return Ok(0);
        }
        let start = turn_starts[t - 1];
        let end = turn_starts.get(t).map_or(usize::MAX, |n| n - 1);
        (start, end)
    } else {
        (1, usize::MAX)
    };

    println!(
        "session {} ({})  [{}]",
        sf.session_id,
        sf.project,
        sf.path.display()
    );
    let mut printed = 0usize;
    let cap = if full { usize::MAX } else { 2000 };
    let malformed = for_each_line(&sf.path, |line_no, l| {
        if line_no < win_start || line_no > win_end {
            return;
        }
        if !scope.line_in_scope(&l) {
            return;
        }
        let ty = l.ty.as_deref().unwrap_or("?");
        if matches!(ty, "file-history-snapshot" | "summary") {
            return;
        }
        let ts = l.timestamp.as_deref().unwrap_or("");
        let side = if l.is_sidechain == Some(true) {
            " [sidechain]"
        } else {
            ""
        };
        println!("#{line_no}  {ts}  {ty}{side}");
        if let Some(s) = l.string_content() {
            println!("    {}", head_of(s, cap));
        }
        for b in l.blocks() {
            match b.ty.as_deref() {
                Some("text") => {
                    if let Some(t) = &b.text {
                        println!("    text: {}", head_of(t, cap));
                    }
                }
                Some("thinking") => {
                    if let Some(t) = &b.thinking {
                        println!("    thinking: {}", head_of(t, cap.min(500)));
                    }
                }
                Some("tool_use") => {
                    let name = b.name.as_deref().unwrap_or("?");
                    let input = b
                        .input
                        .as_ref()
                        .map(std::string::ToString::to_string)
                        .unwrap_or_default();
                    println!("    tool_use[{name}]: {}", head_of(&input, cap));
                }
                Some("tool_result") => {
                    let err = if b.is_error == Some(true) {
                        " (error)"
                    } else {
                        ""
                    };
                    let text = b.content.as_ref().map(tool_result_text).unwrap_or_default();
                    println!("    tool_result{err}: {}", head_of(&text, cap));
                }
                Some(other) => println!("    [{other}]"),
                None => {}
            }
        }
        printed += 1;
    })?;
    note_malformed(malformed);
    Ok(printed)
}

/// Extract a value's string for grep display purposes (used in tests).
#[allow(dead_code)]
fn value_str(v: &Value) -> String {
    v.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_window_is_bounded() {
        let re = Regex::new("needle").unwrap();
        let text = format!("{}needle{}", "a".repeat(200), "b".repeat(200));
        let ctx = match_context(&text, &re).unwrap();
        assert!(ctx.starts_with('…') && ctx.ends_with('…'));
        assert!(ctx.contains("needle"));
        assert!(ctx.chars().count() <= 130);
    }
}
