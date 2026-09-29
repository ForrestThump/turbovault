//! Timed-reminder metadata for Obsidian Tasks.
//!
//! TurboVault parses opt-in and timing fields so a consumer can decide when to
//! fire. It does not send notifications, expand recurrence into occurrences, or
//! run a scheduler.
//!
//! # Opt-in
//!
//! A task is remind-opt-in only when its trailing metadata contains:
//!
//! - the inline tag `#remind` (ASCII case-insensitive; `#reminder` and
//!   `#remind/child` do not count), or
//! - a Dataview field `remind::` or `reminder::` whose value is truthy.
//!
//! Truthy values, after trim and one layer of matching quotes, are `true`,
//! `yes`, `y`, `1`, and `on` (ASCII case-insensitive). Any other value is an
//! explicit opt-out. Rightmost metadata wins.
//!
//! Note-level frontmatter and Full Calendar event notes are not consulted.
//! Full Calendar fields (`date`, `startTime`, `daysOfWeek`, and recurrence
//! bounds) are outside this crate's parser. See the timed-reminders guide.
//!
//! # Vault gate
//!
//! [`crate::VaultConfig::timed_reminders`] defaults to `false`. Parsing still
//! records [`crate::TaskItem::remind`] either way. [`remind_eligible_tasks`]
//! returns nothing while the toggle is off.
//!
//! # Fire time
//!
//! Eligible tasks are open (not done or cancelled) and have at least one of
//! due, scheduled, or start. The preferred anchor is due, then scheduled, then
//! start. A missing clock time means the source was date-only: the consumer
//! chooses the local minute. Recurrence text is passed through unchanged.

use chrono::{NaiveDate, NaiveTime};
use serde::{Deserialize, Serialize};

use crate::models::{SourcePosition, TaskItem};

/// Open, done, or cancelled for reminder filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReminderLifecycle {
    /// Checkbox is not done or cancelled, and no done/cancelled date is set.
    Open,
    /// Checkbox `x` / `X`, [`TaskItem::is_completed`], or a done date.
    Done,
    /// Checkbox `-`, or a cancelled date on a task that is not done.
    Cancelled,
}

/// Which Tasks date a consumer should treat as the reminder anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReminderAnchor {
    Due,
    Scheduled,
    Start,
}

/// A local calendar date plus an optional clock time.
///
/// `time == None` means the source value was date-only. TurboVault does not
/// invent a default minute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalMinute {
    pub date: NaiveDate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<NaiveTime>,
}

/// Preferred local fire anchor for an eligible task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReminderFire {
    pub anchor: ReminderAnchor,
    pub when: LocalMinute,
}

/// Timing and lifecycle projected from one task.
///
/// Present on every task, including ones that are not eligible to fire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskReminderInfo {
    /// Item-level opt-in. Independent of the vault config toggle.
    pub remind: bool,
    pub lifecycle: ReminderLifecycle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<LocalMinute>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled: Option<LocalMinute>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<LocalMinute>,
    /// Tasks recurrence text, for example `every weekday`. Not expanded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recurrence: Option<String>,
}

impl TaskReminderInfo {
    /// Item-level eligibility, ignoring [`crate::VaultConfig::timed_reminders`].
    ///
    /// True when the task opted in, is still open, and has due, scheduled, or start.
    #[must_use]
    pub fn is_eligible(&self) -> bool {
        self.remind && self.lifecycle == ReminderLifecycle::Open && self.primary_when().is_some()
    }

    /// Due, else scheduled, else start. `None` when no anchor date was parsed.
    #[must_use]
    pub fn fire(&self) -> Option<ReminderFire> {
        let when = self.primary_when()?;
        let anchor = if self.due.is_some() {
            ReminderAnchor::Due
        } else if self.scheduled.is_some() {
            ReminderAnchor::Scheduled
        } else {
            ReminderAnchor::Start
        };
        Some(ReminderFire { anchor, when })
    }

    fn primary_when(&self) -> Option<LocalMinute> {
        self.due.or(self.scheduled).or(self.start)
    }
}

/// One remind-eligible task, owned so a consumer can serialize it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemindEligibleTask {
    pub content: String,
    pub position: SourcePosition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_ref: Option<String>,
    pub info: TaskReminderInfo,
    pub fire: ReminderFire,
}

impl TaskItem {
    /// Project reminder fields. Does not apply the vault config toggle.
    #[must_use]
    pub fn reminder_info(&self) -> TaskReminderInfo {
        TaskReminderInfo {
            remind: self.remind,
            lifecycle: self.reminder_lifecycle(),
            due: local_minute(self.due_date, self.due_time),
            scheduled: local_minute(self.scheduled_date, self.scheduled_time),
            start: local_minute(self.start_date, self.start_time),
            recurrence: self.recurrence.clone(),
        }
    }

    /// Open, done, or cancelled.
    ///
    /// Done is checkbox `x`/`X`, [`TaskItem::is_completed`], or a done date.
    /// Cancelled is checkbox `-`, or a cancelled date when the task is not done.
    /// In-progress (`/`) and other custom markers stay open.
    #[must_use]
    pub fn reminder_lifecycle(&self) -> ReminderLifecycle {
        match self.checkbox_marker() {
            'x' | 'X' => ReminderLifecycle::Done,
            '-' => ReminderLifecycle::Cancelled,
            _ if self.is_completed || self.done_date.is_some() => ReminderLifecycle::Done,
            _ if self.cancelled_date.is_some() => ReminderLifecycle::Cancelled,
            _ => ReminderLifecycle::Open,
        }
    }

    /// Item-level remind eligibility, ignoring the vault config toggle.
    #[must_use]
    pub fn is_remind_eligible(&self) -> bool {
        self.reminder_info().is_eligible()
    }

    /// Owned eligible view, or `None` when this task should not fire.
    #[must_use]
    pub fn to_remind_eligible(&self) -> Option<RemindEligibleTask> {
        let info = self.reminder_info();
        if !info.is_eligible() {
            return None;
        }
        let fire = info.fire()?;
        Some(RemindEligibleTask {
            content: self.content.clone(),
            position: self.position,
            id: self.id.clone(),
            block_ref: self.block_ref.clone(),
            info,
            fire,
        })
    }
}

/// Collect remind-eligible tasks when the vault toggle is on.
///
/// Returns an empty vec when `timed_reminders_enabled` is false, including
/// when individual tasks opted in. Pass [`crate::VaultConfig::timed_reminders`].
///
/// ```
/// use turbovault_core::{SourcePosition, TaskBuilder, remind_eligible_tasks};
///
/// let mut builder = TaskBuilder {
///     content: "Stretch 📅 2026-05-01 07:30 #remind".into(),
///     is_completed: false,
///     position: SourcePosition::start(),
///     status_marker: ' ',
/// };
/// let task = builder.build();
/// assert!(remind_eligible_tasks([&task], false).is_empty());
/// let due = remind_eligible_tasks([&task], true);
/// assert_eq!(due.len(), 1);
/// assert!(due[0].fire.when.time.is_some());
/// ```
#[must_use]
pub fn remind_eligible_tasks<'a>(
    tasks: impl IntoIterator<Item = &'a TaskItem>,
    timed_reminders_enabled: bool,
) -> Vec<RemindEligibleTask> {
    if !timed_reminders_enabled {
        return Vec::new();
    }
    tasks
        .into_iter()
        .filter_map(TaskItem::to_remind_eligible)
        .collect()
}

fn local_minute(date: Option<NaiveDate>, time: Option<NaiveTime>) -> Option<LocalMinute> {
    date.map(|date| LocalMinute { date, time })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{SourcePosition, TaskBuilder};
    use chrono::NaiveTime;

    fn item(line_body: &str, marker: char) -> TaskItem {
        let mut builder = TaskBuilder {
            content: line_body.to_string(),
            is_completed: marker == 'x' || marker == 'X',
            position: SourcePosition::new(3, 1, 12, line_body.len()),
            status_marker: marker,
        };
        builder.build()
    }

    fn date(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn unmarked_items_are_skipped() {
        let tasks = vec![
            item("Buy milk 📅 2026-05-01", ' '),
            item("Write report [due:: 2026-05-02] #errands", ' '),
        ];
        assert!(tasks.iter().all(|task| !task.remind));
        assert!(tasks.iter().all(|task| !task.is_remind_eligible()));
        assert!(remind_eligible_tasks(&tasks, true).is_empty());
    }

    #[test]
    fn config_toggle_gates_eligible_tasks() {
        let tasks = vec![item("Stretch 📅 2026-05-01 07:30 #remind", ' ')];
        assert!(tasks[0].is_remind_eligible());
        assert!(remind_eligible_tasks(&tasks, false).is_empty());

        let eligible = remind_eligible_tasks(&tasks, true);
        assert_eq!(eligible.len(), 1);
        assert_eq!(eligible[0].content, "Stretch");
        assert_eq!(eligible[0].fire.anchor, ReminderAnchor::Due);
        assert_eq!(eligible[0].fire.when.date, date("2026-05-01"));
        assert_eq!(
            eligible[0].fire.when.time,
            Some(NaiveTime::from_hms_opt(7, 30, 0).unwrap())
        );
        assert_eq!(eligible[0].position.line, 3);
    }

    #[test]
    fn done_and_cancelled_are_not_eligible() {
        let done = item("Done already 📅 2026-05-01 #remind", 'x');
        let done_upper = item("Done already 📅 2026-05-01 #remind", 'X');
        let cancelled = item("Dropped 📅 2026-05-01 #remind", '-');
        let cancelled_emoji = item("Dropped ❌ 2026-05-01 📅 2026-05-02 #remind", ' ');
        let done_emoji = item("Shipped ✅ 2026-05-01 📅 2026-05-02 #remind", ' ');

        assert_eq!(done.reminder_lifecycle(), ReminderLifecycle::Done);
        assert_eq!(done_upper.reminder_lifecycle(), ReminderLifecycle::Done);
        assert_eq!(cancelled.reminder_lifecycle(), ReminderLifecycle::Cancelled);
        assert_eq!(
            cancelled_emoji.reminder_lifecycle(),
            ReminderLifecycle::Cancelled
        );
        assert_eq!(done_emoji.reminder_lifecycle(), ReminderLifecycle::Done);
        assert!(!done.is_remind_eligible());
        assert!(!cancelled.is_remind_eligible());
        assert!(!cancelled_emoji.is_remind_eligible());
        assert!(!done_emoji.is_remind_eligible());

        let tasks = vec![done, cancelled, cancelled_emoji];
        assert!(remind_eligible_tasks(&tasks, true).is_empty());
    }

    #[test]
    fn in_progress_with_opt_in_stays_eligible() {
        let task = item("Draft ⏳ 2026-05-06 [remind:: true]", '/');
        assert_eq!(task.reminder_lifecycle(), ReminderLifecycle::Open);
        assert!(task.is_remind_eligible());
        let info = task.reminder_info();
        assert_eq!(info.fire().unwrap().anchor, ReminderAnchor::Scheduled);
        assert_eq!(info.scheduled.unwrap().date, date("2026-05-06"));
        assert!(info.scheduled.unwrap().time.is_none());
    }

    #[test]
    fn anchor_precedence_is_due_then_scheduled_then_start() {
        let all = item(
            "Plan [start:: 2026-04-29] [scheduled:: 2026-04-30T08:15] [due:: 2026-05-01] #remind",
            ' ',
        );
        let fire = all.reminder_info().fire().unwrap();
        assert_eq!(fire.anchor, ReminderAnchor::Due);
        assert_eq!(fire.when.date, date("2026-05-01"));
        assert!(fire.when.time.is_none());
        assert_eq!(
            all.scheduled_time,
            Some(NaiveTime::from_hms_opt(8, 15, 0).unwrap())
        );

        let scheduled = item(
            "Plan [start:: 2026-04-29] [scheduled:: 2026-04-30] [reminder:: on]",
            ' ',
        );
        assert_eq!(
            scheduled.reminder_info().fire().unwrap().anchor,
            ReminderAnchor::Scheduled
        );

        let start_only = item("Plan 🛫 2026-04-29 06:00 [remind:: yes]", ' ');
        let fire = start_only.reminder_info().fire().unwrap();
        assert_eq!(fire.anchor, ReminderAnchor::Start);
        assert_eq!(
            fire.when.time,
            Some(NaiveTime::from_hms_opt(6, 0, 0).unwrap())
        );
    }

    #[test]
    fn recurrence_without_a_date_is_not_eligible() {
        let task = item("Habit 🔁 every day #remind", ' ');
        assert!(task.remind);
        assert_eq!(task.recurrence.as_deref(), Some("every day"));
        assert!(task.due_date.is_none());
        assert!(!task.is_remind_eligible());
        assert!(task.reminder_info().fire().is_none());
    }

    #[test]
    fn explicit_remind_false_overrides_nothing_and_stays_ineligible() {
        let task = item("Stretch 📅 2026-05-01 [remind:: false]", ' ');
        assert!(!task.remind);
        assert!(!task.is_remind_eligible());
    }

    #[test]
    fn eligible_view_keeps_recurrence_and_identity() {
        let task = item(
            "Review [due:: 2026-05-01T09:30:15] [repeat:: every weekday] [id:: rev-1] #remind ^rev",
            ' ',
        );
        let eligible = task.to_remind_eligible().unwrap();
        assert_eq!(eligible.id.as_deref(), Some("rev-1"));
        assert_eq!(eligible.block_ref.as_deref(), Some("rev"));
        assert_eq!(eligible.info.recurrence.as_deref(), Some("every weekday"));
        assert_eq!(
            eligible.fire.when.time,
            Some(NaiveTime::from_hms_opt(9, 30, 15).unwrap())
        );
        assert!(eligible.info.remind);
        assert_eq!(eligible.info.lifecycle, ReminderLifecycle::Open);
    }
}
