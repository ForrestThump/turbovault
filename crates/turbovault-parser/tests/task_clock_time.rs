//! Task-parsing regressions for clock-time conventions.
//!
//! Times are written as a clock-face emoji plus `HH:MM` (or a bare `HH:MM`
//! right after a date). Such a token must not clear the date next to it.

use turbovault_parser::parse_tasks;
use turbovault_parser::prelude::TaskItem;

fn due_date(task: &TaskItem) -> Option<String> {
    task.due_date.map(|date| date.to_string())
}

#[test]
fn a_clock_time_does_not_clear_the_due_date() {
    let content =
        "- [ ] Cancel SuperGrok subscription 📅 2026-11-07 🕘 09:00 #task #finance #personal-admin";
    let tasks = parse_tasks(content);

    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].content, "Cancel SuperGrok subscription");
    assert_eq!(due_date(&tasks[0]).as_deref(), Some("2026-11-07"));
    assert_eq!(tasks[0].due_time.as_deref(), Some("09:00"));
    assert_eq!(
        tasks[0].tags,
        vec![
            "task".to_string(),
            "finance".to_string(),
            "personal-admin".to_string()
        ]
    );
}

#[test]
fn a_clock_time_before_the_date_keeps_the_due_date() {
    let content =
        "- [ ] Cancel SuperGrok subscription 🕘 09:00 📅 2026-11-07 #task #finance #personal-admin";
    let tasks = parse_tasks(content);

    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].content, "Cancel SuperGrok subscription");
    assert_eq!(due_date(&tasks[0]).as_deref(), Some("2026-11-07"));
    assert_eq!(tasks[0].due_time.as_deref(), Some("09:00"));
}

#[test]
fn a_bare_time_after_the_date_keeps_the_due_date() {
    let content = "- [ ] Take out the trash 📅 2026-11-14 1:20 #task";
    let tasks = parse_tasks(content);

    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].content, "Take out the trash");
    assert_eq!(due_date(&tasks[0]).as_deref(), Some("2026-11-14"));
    assert_eq!(tasks[0].due_time.as_deref(), Some("01:20"));
    assert_eq!(tasks[0].tags, vec!["task".to_string()]);
}

#[test]
fn a_clock_time_after_the_tags_keeps_the_due_date() {
    let content = "- [ ] Take out the trash 📅 2026-11-07 #task 🕘 09:00";
    let tasks = parse_tasks(content);

    assert_eq!(tasks.len(), 1);
    assert_eq!(due_date(&tasks[0]).as_deref(), Some("2026-11-07"));
    assert_eq!(tasks[0].due_time.as_deref(), Some("09:00"));
    assert_eq!(tasks[0].tags, vec!["task".to_string()]);
}

#[test]
fn a_clock_time_without_a_date_stays_in_the_description() {
    let content = "- [ ] Meet the team 🕘 09:00 #task";
    let tasks = parse_tasks(content);

    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].content, "Meet the team 🕘 09:00");
    assert!(tasks[0].due_date.is_none());
    assert!(tasks[0].due_time.is_none());
    assert_eq!(tasks[0].tags, vec!["task".to_string()]);
}

#[test]
fn a_bare_time_in_prose_is_not_metadata() {
    let content = "- [ ] Meet at 9:00 with the team";
    let tasks = parse_tasks(content);

    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].content, "Meet at 9:00 with the team");
    assert!(tasks[0].due_date.is_none());
    assert!(tasks[0].due_time.is_none());
}
