//! `ccq stats` — one-screen corpus rollup.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::cmds::{extract_prompt, is_complex_command, is_scratch_path};
use crate::out::{human_size, Table};
use crate::scan::{discover_sessions, for_each_line, map_sessions, note_malformed, Scope};

#[derive(Default)]
struct FileStats {
    project: String,
    lines: usize,
    prompts: usize,
    asst_msgs: usize,
    tool_uses: HashMap<String, usize>,
    errors: usize,
    bash: usize,
    bash_complex: usize,
    writes: usize,
    writes_scratch: usize,
}

pub fn stats(scope: &Scope) -> Table {
    let files = discover_sessions(scope);
    let total_size: u64 = files.iter().map(|f| f.size).sum();

    let per_file = map_sessions(&files, |sf| {
        let mut fs = FileStats {
            project: sf.project.clone(),
            ..FileStats::default()
        };
        let malformed = for_each_line(&sf.path, |_, line| {
            if !scope.line_in_scope(&line) {
                return;
            }
            fs.lines += 1;
            match line.ty.as_deref() {
                Some("user") => {
                    if extract_prompt(&line).is_some() {
                        fs.prompts += 1;
                    }
                    for b in line.blocks() {
                        if b.ty.as_deref() == Some("tool_result") && b.is_error == Some(true) {
                            fs.errors += 1;
                        }
                    }
                }
                Some("assistant") => {
                    fs.asst_msgs += 1;
                    for b in line.blocks() {
                        if b.ty.as_deref() != Some("tool_use") {
                            continue;
                        }
                        let name = b.name.as_deref().unwrap_or("?");
                        *fs.tool_uses.entry(name.to_string()).or_insert(0) += 1;
                        match name {
                            "Bash" => {
                                fs.bash += 1;
                                let cmd = b
                                    .input
                                    .as_ref()
                                    .and_then(|i| i.get("command"))
                                    .and_then(Value::as_str)
                                    .unwrap_or("");
                                if is_complex_command(cmd) {
                                    fs.bash_complex += 1;
                                }
                            }
                            "Write" | "Edit" | "NotebookEdit" => {
                                fs.writes += 1;
                                let path = b
                                    .input
                                    .as_ref()
                                    .and_then(|i| {
                                        i.get("file_path").or_else(|| i.get("notebook_path"))
                                    })
                                    .and_then(Value::as_str)
                                    .unwrap_or("");
                                if is_scratch_path(path) {
                                    fs.writes_scratch += 1;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        })?;
        note_malformed(malformed);
        Ok(vec![fs])
    });

    let mut projects: HashSet<&str> = HashSet::new();
    let mut sessions = 0usize;
    let mut lines = 0usize;
    let mut prompts = 0usize;
    let mut asst = 0usize;
    let mut tools: HashMap<String, usize> = HashMap::new();
    let mut errors = 0usize;
    let (mut bash, mut bash_complex, mut writes, mut writes_scratch) = (0, 0, 0, 0);
    for fs in &per_file {
        if fs.lines == 0 {
            continue;
        }
        projects.insert(&fs.project);
        sessions += 1;
        lines += fs.lines;
        prompts += fs.prompts;
        asst += fs.asst_msgs;
        errors += fs.errors;
        bash += fs.bash;
        bash_complex += fs.bash_complex;
        writes += fs.writes;
        writes_scratch += fs.writes_scratch;
        for (k, v) in &fs.tool_uses {
            *tools.entry(k.clone()).or_insert(0) += v;
        }
    }
    let total_tools: usize = tools.values().sum();
    let mut top_tools: Vec<(String, usize)> = tools.into_iter().collect();
    top_tools.sort_by_key(|(_, n)| std::cmp::Reverse(*n));

    let pct = |num: usize, den: usize| -> String {
        if den == 0 {
            "0%".to_string()
        } else {
            format!("{:.0}%", num as f64 * 100.0 / den as f64)
        }
    };

    let mut rows = vec![
        vec!["projects".into(), projects.len().to_string()],
        vec!["sessions".into(), sessions.to_string()],
        vec!["size".into(), human_size(total_size)],
        vec!["lines".into(), lines.to_string()],
        vec!["user_prompts".into(), prompts.to_string()],
        vec!["assistant_msgs".into(), asst.to_string()],
        vec!["tool_uses".into(), total_tools.to_string()],
        vec!["tool_errors".into(), errors.to_string()],
        vec!["bash_commands".into(), bash.to_string()],
        vec![
            "bash_complex".into(),
            format!("{bash_complex} ({})", pct(bash_complex, bash)),
        ],
        vec!["writes".into(), writes.to_string()],
        vec![
            "writes_scratch".into(),
            format!("{writes_scratch} ({})", pct(writes_scratch, writes)),
        ],
    ];
    for (name, count) in top_tools.into_iter().take(10) {
        rows.push(vec![format!("tool:{name}"), count.to_string()]);
    }
    Table {
        headers: vec!["metric", "value"],
        rows,
    }
}
