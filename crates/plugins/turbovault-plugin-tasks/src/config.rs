//! Self-tuning configuration for the `tasks` module.
//!
//! The Obsidian Tasks plugin records two things this module needs in order to
//! behave like the user's own vault: the **metadata format** (emoji vs Dataview
//! inline fields) it writes, and the **global filter** — a tag that marks which
//! checkbox lines are "real" tasks. Both live in
//! `.obsidian/plugins/obsidian-tasks-plugin/data.json`.
//!
//! We resolve the config in priority order:
//! 1. the authoritative `data.json`, read through the curated
//!    [`VaultApi::read_config`](turbovault_plugin_api::VaultApi::read_config);
//! 2. a **content heuristic** over the vault's existing task lines, when the
//!    settings file is absent or the host declines config reads;
//! 3. a plain default (emoji, no global filter).

use serde_json::Value;

/// Which trailing-metadata dialect the user's Tasks plugin reads and writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskFormat {
    /// `📅 2026-07-19 ⏫ 🔁 every week` — the Tasks plugin's emoji signifiers.
    Emoji,
    /// `[due:: 2026-07-19] [priority:: high]` — Dataview inline fields.
    Dataview,
}

impl Default for TaskFormat {
    fn default() -> Self {
        // Matches the Tasks plugin's own default (`tasksPluginEmoji`).
        TaskFormat::Emoji
    }
}

/// Where a resolved [`TasksConfig`] came from — surfaced by `tasks_config` so
/// the sync is observable rather than magic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    /// Parsed from the Tasks plugin's `data.json`.
    ObsidianData,
    /// Inferred from the vault's existing task lines.
    Heuristic,
    /// Nothing to go on; built-in defaults.
    Default,
}

impl ConfigSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ConfigSource::ObsidianData => "obsidian-data",
            ConfigSource::Heuristic => "heuristic",
            ConfigSource::Default => "default",
        }
    }
}

/// Resolved Tasks-plugin settings the module tunes itself to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TasksConfig {
    pub format: TaskFormat,
    /// Global-filter tag *with* its leading `#` (e.g. `#task`), if the user set
    /// one. When present, only checkbox lines bearing this tag are treated as
    /// tasks — mirroring the Tasks plugin.
    pub global_filter: Option<String>,
    /// Whether the Tasks plugin hides the global-filter tag from the rendered
    /// description.
    pub remove_global_filter: bool,
    pub source: ConfigSource,
}

impl Default for TasksConfig {
    fn default() -> Self {
        Self {
            format: TaskFormat::default(),
            global_filter: None,
            remove_global_filter: false,
            source: ConfigSource::Default,
        }
    }
}

/// Canonical vault-relative path of the Tasks plugin settings file.
pub const OBSIDIAN_TASKS_DATA_PATH: &str = ".obsidian/plugins/obsidian-tasks-plugin/data.json";

impl TasksConfig {
    /// Parse the Tasks plugin's `data.json`. Returns `None` if the bytes are not
    /// the JSON object we expect, so the caller can fall back to heuristics.
    pub fn from_obsidian_data(bytes: &[u8]) -> Option<TasksConfig> {
        let value: Value = serde_json::from_slice(bytes).ok()?;
        let object = value.as_object()?;

        // `taskFormat` was introduced with Dataview support; older installs omit
        // it and are emoji by definition.
        let format = match object.get("taskFormat").and_then(Value::as_str) {
            Some("dataview") => TaskFormat::Dataview,
            _ => TaskFormat::Emoji,
        };

        let global_filter = object
            .get("globalFilter")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|filter| !filter.is_empty())
            .map(str::to_string);

        let remove_global_filter = object
            .get("removeGlobalFilter")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        Some(TasksConfig {
            format,
            global_filter,
            remove_global_filter,
            source: ConfigSource::ObsidianData,
        })
    }

    /// Best-effort config inferred from raw task-bearing note text when the
    /// settings file is unavailable. Only the format is inferable; a global
    /// filter that is not already on tasks cannot be discovered this way.
    pub fn from_heuristic(sample: &str) -> TasksConfig {
        match detect_format(sample) {
            Some(format) => TasksConfig {
                format,
                global_filter: None,
                remove_global_filter: false,
                source: ConfigSource::Heuristic,
            },
            None => TasksConfig::default(),
        }
    }
}

/// The emoji signifiers the parser recognizes; their presence implies emoji
/// format.
const FORMAT_EMOJIS: [&str; 7] = ["📅", "⏳", "🛫", "➕", "✅", "❌", "🔁"];

/// Infer the metadata dialect by counting emoji signifiers against Dataview
/// inline fields across the sample. `None` when there is no signal either way.
fn detect_format(sample: &str) -> Option<TaskFormat> {
    let emoji = FORMAT_EMOJIS
        .iter()
        .map(|marker| sample.matches(marker).count())
        .sum::<usize>();

    let dataview = ["[due::", "[scheduled::", "[start::", "[created::", "[completion::",
        "[priority::", "[repeat::", "[recurrence::"]
        .iter()
        .map(|marker| sample.matches(marker).count())
        .sum::<usize>();

    match (emoji, dataview) {
        (0, 0) => None,
        (e, d) if d > e => Some(TaskFormat::Dataview),
        _ => Some(TaskFormat::Emoji),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_emoji_default_and_global_filter_from_data_json() {
        let json = br##"{ "globalFilter": "#task", "removeGlobalFilter": true }"##;
        let config = TasksConfig::from_obsidian_data(json).expect("parse");
        assert_eq!(config.format, TaskFormat::Emoji);
        assert_eq!(config.global_filter.as_deref(), Some("#task"));
        assert!(config.remove_global_filter);
        assert_eq!(config.source, ConfigSource::ObsidianData);
    }

    #[test]
    fn parses_dataview_format() {
        let json = br#"{ "taskFormat": "dataview", "globalFilter": "" }"#;
        let config = TasksConfig::from_obsidian_data(json).expect("parse");
        assert_eq!(config.format, TaskFormat::Dataview);
        assert_eq!(config.global_filter, None);
    }

    #[test]
    fn rejects_non_json() {
        assert!(TasksConfig::from_obsidian_data(b"not json").is_none());
    }

    #[test]
    fn heuristic_detects_dataview_over_emoji() {
        let sample = "- [ ] a [due:: 2026-01-01]\n- [ ] b [priority:: high]";
        assert_eq!(TasksConfig::from_heuristic(sample).format, TaskFormat::Dataview);
        assert_eq!(
            TasksConfig::from_heuristic(sample).source,
            ConfigSource::Heuristic
        );
    }

    #[test]
    fn heuristic_detects_emoji() {
        let sample = "- [ ] a 📅 2026-01-01 ⏫";
        assert_eq!(TasksConfig::from_heuristic(sample).format, TaskFormat::Emoji);
    }

    #[test]
    fn heuristic_falls_back_to_default_without_signal() {
        assert_eq!(TasksConfig::from_heuristic("- [ ] plain task").source, ConfigSource::Default);
    }
}
