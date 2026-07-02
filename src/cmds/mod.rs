//! Subcommand implementations and shared extraction heuristics.

pub mod agents;
pub mod inspect;
pub mod listing;
pub mod stats;

use crate::model::RawLine;

/// Extract a genuine human prompt from a `user`-typed line, or `None` when
/// the line is a `tool_result` carrier, a command wrapper, or reminder-only.
pub fn extract_prompt(line: &RawLine) -> Option<String> {
    if line.ty.as_deref() != Some("user") {
        return None;
    }
    let role = line.message.as_ref().and_then(|m| m.role.as_deref());
    if role.is_some_and(|r| r != "user") {
        return None;
    }
    let text = if let Some(s) = line.string_content() {
        s.to_string()
    } else {
        let blocks = line.blocks();
        if blocks.is_empty()
            || blocks
                .iter()
                .any(|b| b.ty.as_deref() == Some("tool_result"))
        {
            return None;
        }
        let joined: Vec<&str> = blocks
            .iter()
            .filter(|b| b.ty.as_deref() == Some("text"))
            .filter_map(|b| b.text.as_deref())
            .collect();
        joined.join("\n")
    };
    if text.contains("<command-name>") || text.contains("<local-command-caveat>") {
        return None;
    }
    // Harness-injected notifications (background task/agent completions)
    // arrive as user lines but are not human prompts.
    if text.trim_start().starts_with("<task-notification>") {
        return None;
    }
    let stripped = strip_tag_spans(&text, "system-reminder");
    let trimmed = stripped.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

/// Remove `<tag>…</tag>` spans (non-nested) from `text`.
pub fn strip_tag_spans(text: &str, tag: &str) -> String {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        match rest.find(&open) {
            Some(start) => {
                out.push_str(&rest[..start]);
                let after = &rest[start + open.len()..];
                match after.find(&close) {
                    Some(end) => rest = &after[end + close.len()..],
                    None => break, // unclosed: drop the remainder
                }
            }
            None => {
                out.push_str(rest);
                break;
            }
        }
    }
    out
}

/// Heuristic for `ccq bash --complex`: one-off scripting rather than a
/// plain command invocation.
pub fn is_complex_command(cmd: &str) -> bool {
    if cmd.contains('\n') || cmd.contains("<<") {
        return true;
    }
    for needle in ["python3 -c", "python -c", "node -e", "ruby -e", "perl -e"] {
        if cmd.contains(needle) {
            return true;
        }
    }
    // A jq/awk/sed/for/while with an inline program body.
    for tool in ["jq ", "awk ", "sed "] {
        if let Some(pos) = cmd.find(tool) {
            if cmd[pos..].contains('\'') {
                return true;
            }
        }
    }
    let head = cmd.trim_start();
    if head.starts_with("for ") || head.starts_with("while ") {
        return true;
    }
    cmd.contains("; for ") || cmd.contains("| while ") || cmd.contains("&& for ")
}

/// Normalize an error message into a recurrence signature: first line,
/// paths and numbers stripped, whitespace collapsed, capped at 120 chars.
pub fn error_signature(text: &str) -> String {
    let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let mut sig = String::with_capacity(first.len());
    let mut in_path = false;
    let mut in_num = false;
    for c in first.chars() {
        if in_path {
            if c.is_whitespace() || c == ':' || c == '\'' || c == '"' || c == ')' {
                in_path = false;
                sig.push(c);
            }
            continue;
        }
        if c == '/' || c == '~' {
            in_path = true;
            in_num = false;
            sig.push_str("<path>");
            continue;
        }
        if c.is_ascii_digit() {
            if !in_num {
                sig.push('N');
                in_num = true;
            }
            continue;
        }
        in_num = false;
        sig.push(c);
    }
    let collapsed: String = sig.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > 120 {
        let cut: String = collapsed.chars().take(120).collect();
        format!("{cut}…")
    } else {
        collapsed
    }
}

/// Is this a scratch/temp path? (`ccq writes --scratch`)
pub fn is_scratch_path(path: &str) -> bool {
    path.starts_with("/tmp/")
        || path.starts_with("/private/tmp/")
        || path.starts_with("/var/tmp/")
        || path.contains("scratchpad")
}

/// Short human duration between two timestamps.
pub fn human_duration(secs: i64) -> String {
    if secs < 0 {
        return "-".to_string();
    }
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h}h{m:02}m")
    } else if m > 0 {
        format!("{m}m{s:02}s")
    } else {
        format!("{s}s")
    }
}

/// First `n` chars of `s`, flattened to one line.
pub fn head_of(s: &str, n: usize) -> String {
    let flat = s.replace('\n', "⏎");
    let mut out: String = flat.chars().take(n).collect();
    if flat.chars().count() > n {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complex_heuristic() {
        assert!(is_complex_command("for f in *; do echo $f; done"));
        assert!(is_complex_command("python3 -c 'print(1)'"));
        assert!(is_complex_command("cat <<EOF\nhi\nEOF"));
        assert!(is_complex_command("cat x | jq '.foo[] | select(.a)'"));
        assert!(is_complex_command("line1\nline2"));
        assert!(!is_complex_command("cargo build --release"));
        assert!(!is_complex_command("git status"));
        assert!(!is_complex_command("ls -la /tmp"));
    }

    #[test]
    fn signatures_normalize_paths_and_numbers() {
        assert_eq!(
            error_signature("Error: /Users/x/foo/bar.rs:42: expected 3 args"),
            "Error: <path>:N: expected N args"
        );
        assert_eq!(error_signature("exit code 127"), "exit code N");
        assert_eq!(
            error_signature("\n\nPermission denied"),
            "Permission denied"
        );
    }

    #[test]
    fn strip_reminder_spans() {
        assert_eq!(
            strip_tag_spans(
                "hi <system-reminder>x</system-reminder> there",
                "system-reminder"
            ),
            "hi  there"
        );
        assert_eq!(
            strip_tag_spans("<system-reminder>only</system-reminder>", "system-reminder"),
            ""
        );
    }

    #[test]
    fn scratch_paths() {
        assert!(is_scratch_path("/tmp/x.py"));
        assert!(is_scratch_path("/private/tmp/claude-501/s/scratchpad/y"));
        assert!(!is_scratch_path("/Users/x/project/src/main.rs"));
    }
}
