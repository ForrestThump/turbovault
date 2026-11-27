//! Task-parsing regressions for list nesting.
//!
//! The Obsidian Tasks plugin is line-oriented: a task is one Markdown line,
//! and a following (indented) list item is a separate item that never rewrites
//! the task above it. TurboVault parses whole documents, so the document parser
//! has to keep that guarantee.

use turbovault_parser::parse_tasks;
use turbovault_parser::prelude::TaskItem;

fn due_date(task: &TaskItem) -> Option<String> {
    task.due_date.map(|date| date.to_string())
}

#[test]
fn a_plain_nested_bullet_does_not_join_the_parent_task() {
    let content = "- [ ] Order eyeglasses 📅 2026-12-31 #task #health #personal\n    - 📝 Prescription is somewhere in the house";
    let tasks = parse_tasks(content);

    assert_eq!(tasks.len(), 1);
    let parent = &tasks[0];
    assert_eq!(parent.content, "Order eyeglasses");
    assert!(
        !parent.content.contains("Prescription"),
        "the nested bullet leaked into the parent: {parent:?}"
    );
    assert_eq!(due_date(parent).as_deref(), Some("2026-12-31"));
    assert!(parent.tags.iter().any(|tag| tag == "task"));
    assert_eq!(parent.position.line, 1);
}

#[test]
fn a_nested_checkbox_becomes_its_own_task() {
    let content = "- [ ] Order eyeglasses 📅 2026-12-31 #task #health #personal\n    - [ ] Find prescription #task";
    let tasks = parse_tasks(content);

    assert_eq!(tasks.len(), 2);

    let parent = &tasks[0];
    assert_eq!(parent.content, "Order eyeglasses");
    assert_eq!(due_date(parent).as_deref(), Some("2026-12-31"));
    assert!(parent.tags.iter().any(|tag| tag == "task"));
    assert_eq!(parent.position.line, 1);

    let child = &tasks[1];
    assert_eq!(child.content, "Find prescription");
    assert!(
        !child.content.contains("Order eyeglasses"),
        "the parent leaked into the child: {child:?}"
    );
    assert_eq!(child.position.line, 2);
}

#[test]
fn a_sibling_bullet_leaves_the_parent_task_alone() {
    let content = "- [ ] Order eyeglasses 📅 2026-12-31 #task #health #personal\n- 📝 Prescription is somewhere in the house";
    let tasks = parse_tasks(content);

    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].content, "Order eyeglasses");
    assert_eq!(due_date(&tasks[0]).as_deref(), Some("2026-12-31"));
}

#[test]
fn a_deeper_nested_bullet_does_not_join_the_parent_task() {
    let content = "- [ ] Order eyeglasses 📅 2026-12-31 #task #health #personal\n  - 📝 Prescription is somewhere in the house";
    let tasks = parse_tasks(content);

    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].content, "Order eyeglasses");
    assert_eq!(due_date(&tasks[0]).as_deref(), Some("2026-12-31"));
}
