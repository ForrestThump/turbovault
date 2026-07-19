//! `tasks` — Obsidian-tasks vertical for TurboVault, as a compiled-in plugin module.
//!
//! This module extracts the historical task MCP tools (originally written on the
//! monolithic `tools.rs` against internal managers) and rebuilds them on the
//! stable plugin boundary. All vault access goes through the curated
//! [`VaultApi`]; writes are compare-and-swap only (`Match(version)`), so the
//! module can never blind-overwrite a note.
//!
//! Local tool names are unprefixed here; the host advertises them as
//! `tasks_<name>` (e.g. `tasks_list`, `tasks_complete`).
//!
//! # Tool surface (v1)
//!
//! | local name | kind | notes |
//! |---|---|---|
//! | `list`     | read  | all/pending/completed, optional tag filter |
//! | `overdue`  | read  | pending tasks past `due_date`, oldest first |
//! | `tags`     | read  | distinct inline task tags across the vault |
//! | `complete` | write | flip the checkbox + stamp `✅ <done>` (CAS) |
//! | `delete`   | write | remove the task line (CAS) |
//!
//! `update` (arbitrary field edits) and recurrence-spawn on `complete` are
//! deliberately deferred: they need the `TaskItem` → markdown renderer
//! (`to_markdown_line`), which lives on the fork's task branch and should be
//! ported into `turbovault-core` (or this crate) as a follow-up rather than
//! re-implemented lossily here.

use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::NaiveDate;
use serde_json::{Value, json};
use turbomcp_types::ToolInputSchema;
use turbovault_core::TaskItem;
use turbovault_plugin_api::{
    Plugin, PluginContext, PluginDescriptor, PluginError, PluginProvider, PluginRequestContext,
    PluginResult, Tool, ToolResult, VaultApi, WriteNoteRequest, WritePrecondition, WriteProvenance,
};

/// Compiled-in factory for the `tasks` module.
pub struct TasksPlugin;

impl Plugin for TasksPlugin {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            id: "tasks".to_string(),
            name: "Tasks".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            description: "Obsidian-tasks queries and edits over the active vault".to_string(),
        }
    }

    fn build(&self, context: PluginContext) -> PluginResult<Arc<dyn PluginProvider>> {
        Ok(Arc::new(TasksProvider {
            vault: context.vault,
        }))
    }
}

/// Runtime provider holding the curated vault facade.
pub struct TasksProvider {
    vault: VaultApi,
}

#[async_trait]
impl PluginProvider for TasksProvider {
    fn tools(&self) -> Vec<Tool> {
        vec![
            tool(
                "list",
                "List tasks across the active vault, optionally filtered by status and tags",
                json!({
                    "type": "object",
                    "properties": {
                        "status": {
                            "type": "string",
                            "description": "all | pending | completed (default all)",
                            "enum": ["all", "pending", "open", "todo", "incomplete", "completed", "done"]
                        },
                        "tags": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Only tasks bearing all of these inline tags (leading # optional)"
                        }
                    }
                }),
            ),
            tool(
                "overdue",
                "List pending tasks whose due date is before today (or a given date), oldest first",
                json!({
                    "type": "object",
                    "properties": {
                        "as_of": {
                            "type": "string",
                            "description": "YYYY-MM-DD; overdue is measured against this date (default today)"
                        }
                    }
                }),
            ),
            tool(
                "tags",
                "Return the deduplicated, sorted set of inline #tags used on task items",
                json!({ "type": "object", "properties": {} }),
            ),
            tool(
                "complete",
                "Mark a task complete: flip its checkbox to [x] and stamp today's done date",
                json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Vault-relative note path (from list)" },
                        "line": { "type": "integer", "minimum": 1, "description": "1-based line of the task (from list)" },
                        "expected_version": { "type": "string", "description": "Optimistic-concurrency token from a prior read; rejected if the note has changed" },
                        "done_date": { "type": "string", "description": "YYYY-MM-DD to stamp instead of today" }
                    },
                    "required": ["path", "line"]
                }),
            ),
            tool(
                "delete",
                "Permanently remove a task line from a note (destructive; use complete to record completion instead)",
                json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Vault-relative note path (from list)" },
                        "line": { "type": "integer", "minimum": 1, "description": "1-based line of the task (from list)" },
                        "expected_version": { "type": "string", "description": "Optimistic-concurrency token from a prior read; rejected if the note has changed" }
                    },
                    "required": ["path", "line"]
                }),
            ),
        ]
    }

    async fn call_tool(
        &self,
        name: &str,
        arguments: Value,
        context: PluginRequestContext,
    ) -> PluginResult<ToolResult> {
        match name {
            "list" => self.list(&arguments).await,
            "overdue" => self.overdue(&arguments).await,
            "tags" => self.tags().await,
            "complete" => self.complete(&arguments, &context).await,
            "delete" => self.delete(&arguments, &context).await,
            other => Err(PluginError::not_found(format!("unknown tool {other:?}"))),
        }
    }
}

impl TasksProvider {
    /// Read every markdown note and parse its tasks, tagged with the note path.
    async fn read_all_tasks(&self) -> PluginResult<Vec<(String, TaskItem)>> {
        let notes = self.vault.list_notes().await?;
        let mut out = Vec::new();
        for path in notes {
            if !path.ends_with(".md") {
                continue;
            }
            // A note that fails to read (e.g. vanished between list and read) is
            // skipped rather than failing the whole query.
            let Ok(snapshot) = self.vault.read_note(&path).await else {
                continue;
            };
            for task in turbovault_parser::parse_tasks(&snapshot.content) {
                out.push((path.clone(), task));
            }
        }
        Ok(out)
    }

    async fn list(&self, args: &Value) -> PluginResult<ToolResult> {
        let status = get_str(args, "status")
            .unwrap_or("all")
            .to_ascii_lowercase();
        let tag_filters: Vec<String> = args
            .get("tags")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|tag| tag.trim_start_matches('#').to_ascii_lowercase())
                    .filter(|tag| !tag.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let mut tasks = Vec::new();
        for (path, task) in self.read_all_tasks().await? {
            let passes_status = match status.as_str() {
                "completed" | "done" => task.is_completed,
                "pending" | "open" | "todo" | "incomplete" => !task.is_completed,
                _ => true,
            };
            if !passes_status {
                continue;
            }
            let passes_tags = tag_filters.iter().all(|required| {
                task.tags
                    .iter()
                    .any(|tag| tag.eq_ignore_ascii_case(required))
            });
            if passes_tags {
                tasks.push(task_json(&path, &task));
            }
        }

        let count = tasks.len();
        ok_json(json!({
            "tasks": tasks,
            "count": count,
            "status_filter": status,
            "tag_filters": tag_filters,
        }))
    }

    async fn overdue(&self, args: &Value) -> PluginResult<ToolResult> {
        let as_of = match get_str(args, "as_of") {
            Some(raw) if !raw.is_empty() => parse_date(raw, "as_of")?,
            _ => chrono::Local::now().date_naive(),
        };

        let mut overdue = Vec::new();
        for (path, task) in self.read_all_tasks().await? {
            if task.is_completed {
                continue;
            }
            if let Some(due) = task.due_date
                && due < as_of
            {
                let mut entry = task_json(&path, &task);
                entry["days_overdue"] = json!((as_of - due).num_days());
                overdue.push(entry);
            }
        }
        overdue.sort_by(|a, b| {
            a["due_date"]
                .as_str()
                .unwrap_or("")
                .cmp(b["due_date"].as_str().unwrap_or(""))
        });

        let count = overdue.len();
        ok_json(json!({
            "tasks": overdue,
            "count": count,
            "as_of": as_of.to_string(),
        }))
    }

    async fn tags(&self) -> PluginResult<ToolResult> {
        let mut set: BTreeSet<String> = BTreeSet::new();
        for (_, task) in self.read_all_tasks().await? {
            for tag in task.tags {
                set.insert(tag.to_ascii_lowercase());
            }
        }
        let tags: Vec<&String> = set.iter().collect();
        let count = tags.len();
        ok_json(json!({ "tags": tags, "count": count }))
    }

    async fn complete(
        &self,
        args: &Value,
        context: &PluginRequestContext,
    ) -> PluginResult<ToolResult> {
        let path = required_str(args, "path")?;
        let line = required_line(args)?;
        let done = match get_str(args, "done_date") {
            Some(raw) if !raw.is_empty() => parse_date(raw, "done_date")?,
            _ => chrono::Local::now().date_naive(),
        };

        let snapshot = self.vault.read_note(path).await?;
        verify_version(args, &snapshot.version)?;
        let task = task_at_line(&snapshot.content, line, path)?;

        let separator = line_sep(&snapshot.content);
        let mut lines: Vec<String> = snapshot.content.split(separator).map(str::to_string).collect();
        let idx = line - 1;
        if idx >= lines.len() {
            return Err(PluginError::invalid_input(format!(
                "line {line} is out of range for {path:?}"
            )));
        }
        let completed_line = mark_line_completed(&lines[idx], done)?;
        lines[idx] = completed_line.clone();
        let new_content = lines.join(separator);

        let receipt = self
            .vault
            .write_note(WriteNoteRequest {
                path: path.to_string(),
                content: new_content,
                precondition: WritePrecondition::Match(snapshot.version.clone()),
                commit_message: Some(format!("tasks: complete {path}:{line}")),
                provenance: Some(provenance(context, format!("complete {path}:{line}"))),
            })
            .await?;

        ok_json(json!({
            "path": path,
            "line": line,
            "completed_line": completed_line,
            "done_date": done.to_string(),
            "version": receipt.version,
            "task": task_json(path, &task),
        }))
    }

    async fn delete(
        &self,
        args: &Value,
        context: &PluginRequestContext,
    ) -> PluginResult<ToolResult> {
        let path = required_str(args, "path")?;
        let line = required_line(args)?;

        let snapshot = self.vault.read_note(path).await?;
        verify_version(args, &snapshot.version)?;
        let task = task_at_line(&snapshot.content, line, path)?;

        let separator = line_sep(&snapshot.content);
        let mut lines: Vec<String> = snapshot.content.split(separator).map(str::to_string).collect();
        let idx = line - 1;
        if idx >= lines.len() {
            return Err(PluginError::invalid_input(format!(
                "line {line} is out of range for {path:?}"
            )));
        }
        lines.remove(idx);
        let new_content = lines.join(separator);

        let receipt = self
            .vault
            .write_note(WriteNoteRequest {
                path: path.to_string(),
                content: new_content,
                precondition: WritePrecondition::Match(snapshot.version.clone()),
                commit_message: Some(format!("tasks: delete {path}:{line}")),
                provenance: Some(provenance(context, format!("delete {path}:{line}"))),
            })
            .await?;

        ok_json(json!({
            "path": path,
            "line": line,
            "deleted_content": task.content,
            "version": receipt.version,
        }))
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn tool(name: &str, description: &str, schema: Value) -> Tool {
    Tool::new(name, description).with_schema(ToolInputSchema::from_value(schema))
}

fn ok_json(value: Value) -> PluginResult<ToolResult> {
    ToolResult::json(&value).map_err(|error| PluginError::internal(error.to_string()))
}

fn get_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn required_str<'a>(args: &'a Value, key: &str) -> PluginResult<&'a str> {
    get_str(args, key).ok_or_else(|| PluginError::invalid_input(format!("{key} is required")))
}

fn required_line(args: &Value) -> PluginResult<usize> {
    args.get("line")
        .and_then(Value::as_u64)
        .filter(|n| *n >= 1)
        .map(|n| n as usize)
        .ok_or_else(|| PluginError::invalid_input("line is required and must be >= 1"))
}

fn verify_version(args: &Value, actual: &str) -> PluginResult<()> {
    if let Some(expected) = get_str(args, "expected_version")
        && expected != actual
    {
        return Err(PluginError::conflict(
            "expected_version does not match the current note; re-read before editing",
        ));
    }
    Ok(())
}

fn parse_date(raw: &str, field: &str) -> PluginResult<NaiveDate> {
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .map_err(|_| PluginError::invalid_input(format!("invalid {field} {raw:?} — use YYYY-MM-DD")))
}

fn provenance(context: &PluginRequestContext, note: String) -> WriteProvenance {
    WriteProvenance {
        source: "tasks-plugin".to_string(),
        correlation_id: Some(context.request_id.clone()),
        note: Some(note),
    }
}

/// `"\r\n"` if the note uses Windows line endings, else `"\n"`.
fn line_sep(content: &str) -> &'static str {
    if content.contains("\r\n") { "\r\n" } else { "\n" }
}

/// Find the parsed task on `line` (1-based), erroring if none is there.
fn task_at_line(content: &str, line: usize, path: &str) -> PluginResult<TaskItem> {
    turbovault_parser::parse_tasks(content)
        .into_iter()
        .find(|task| task.position.line == line)
        .ok_or_else(|| PluginError::invalid_input(format!("no task at {path}:{line}")))
}

/// Flip a task line's checkbox to `[x]` and append `✅ <done>` if absent.
///
/// Operates directly on the source line so no task metadata is lost to a
/// round-trip through `TaskItem`.
fn mark_line_completed(line: &str, done: NaiveDate) -> PluginResult<String> {
    let bytes = line.as_bytes();
    let mut checkbox = None;
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 2] == b']' {
            checkbox = Some(i);
            break;
        }
        i += 1;
    }
    let open = checkbox.ok_or_else(|| {
        PluginError::invalid_input(format!("line is not a task (no checkbox): {line:?}"))
    })?;
    let status = open + 1;

    let mut result = String::with_capacity(line.len() + 16);
    result.push_str(&line[..status]);
    result.push('x');
    result.push_str(&line[status + 1..]);
    if !result.contains('✅') {
        result.push_str(&format!(" ✅ {done}"));
    }
    Ok(result)
}

fn task_json(path: &str, task: &TaskItem) -> Value {
    json!({
        "path": path,
        "line": task.position.line,
        "content": task.content,
        "is_completed": task.is_completed,
        "priority": task.priority,
        "due_date": task.due_date.map(|date| date.to_string()),
        "scheduled_date": task.scheduled_date.map(|date| date.to_string()),
        "start_date": task.start_date.map(|date| date.to_string()),
        "done_date": task.done_date.map(|date| date.to_string()),
        "recurrence": task.recurrence,
        "tags": task.tags,
        "depends_on": task.depends_on,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_line_separator() {
        assert_eq!(line_sep("a\nb"), "\n");
        assert_eq!(line_sep("a\r\nb"), "\r\n");
    }

    #[test]
    fn completes_a_plain_task_line() {
        let out = mark_line_completed("- [ ] buy milk", "2026-07-18".parse().unwrap()).unwrap();
        assert_eq!(out, "- [x] buy milk ✅ 2026-07-18");
    }

    #[test]
    fn completes_an_indented_task_and_preserves_metadata() {
        let out =
            mark_line_completed("    - [ ] pay rent 📅 2026-07-01", "2026-07-18".parse().unwrap())
                .unwrap();
        assert_eq!(out, "    - [x] pay rent 📅 2026-07-01 ✅ 2026-07-18");
    }

    #[test]
    fn does_not_double_stamp_done_date() {
        let out = mark_line_completed(
            "- [/] in progress ✅ 2026-07-10",
            "2026-07-18".parse().unwrap(),
        )
        .unwrap();
        assert_eq!(out, "- [x] in progress ✅ 2026-07-10");
    }

    #[test]
    fn rejects_a_non_task_line() {
        assert!(mark_line_completed("just a paragraph", "2026-07-18".parse().unwrap()).is_err());
    }
}
