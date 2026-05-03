//! Integration tests for TurboVault Server

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use tempfile::TempDir;
    use tokio::fs;
    use turbovault::ObsidianMcpServer;
    use turbovault_core::{ConfigProfile, TaskPriority, VaultConfig};
    use turbovault_vault::VaultManager;

    /// Helper to create a test vault
    async fn create_test_vault() -> (TempDir, VaultManager) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let vault_path = temp_dir.path();

        // Create some test files
        fs::write(
            vault_path.join("index.md"),
            "# Index\n\n[[note1]], [[note2]]",
        )
        .await
        .expect("Failed to write index.md");

        fs::write(
            vault_path.join("note1.md"),
            "# Note 1\n\nThis links to [[note2]]",
        )
        .await
        .expect("Failed to write note1.md");

        fs::write(
            vault_path.join("note2.md"),
            "# Note 2\n\nThis links back to [[note1]] and [[index]]",
        )
        .await
        .expect("Failed to write note2.md");

        // Create vault manager
        let mut config = ConfigProfile::Development.create_config();
        let vault_config = VaultConfig::builder("default", vault_path)
            .build()
            .expect("Failed to create vault config");
        config.vaults.push(vault_config);

        let manager = VaultManager::new(config).expect("Failed to create vault manager");
        manager
            .initialize()
            .await
            .expect("Failed to initialize vault");

        (temp_dir, manager)
    }

    #[tokio::test]
    async fn test_server_creation() {
        let _server = ObsidianMcpServer::new();
        // Server should be creatable without vault (no assertion needed)
    }

    #[tokio::test]
    async fn test_server_initialization() {
        let (_temp, _manager) = create_test_vault().await;
        let _server = ObsidianMcpServer::new().expect("Failed to create server");
        // Server should initialize without vault (vault-agnostic design, no assertion needed)
    }

    #[tokio::test]
    async fn test_vault_path_resolution() {
        let (temp_dir, manager) = create_test_vault().await;
        let expected = temp_dir.path();
        let actual = manager.vault_path();
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_scan_vault() {
        let (_temp, manager) = create_test_vault().await;
        let files = manager.scan_vault().await.expect("Failed to scan vault");
        assert!(files.len() >= 3, "Should find at least 3 markdown files");
    }

    #[tokio::test]
    async fn test_parse_file() {
        let (_temp, manager) = create_test_vault().await;
        let vault_file = manager
            .parse_file(&PathBuf::from("index.md"))
            .await
            .expect("Failed to parse file");
        assert_eq!(vault_file.path.file_name().unwrap(), "index.md");
    }

    #[tokio::test]
    async fn test_link_graph_access() {
        let (_temp, manager) = create_test_vault().await;
        let graph = manager.link_graph();
        let _read_guard = graph.read().await;
        // Should be able to acquire read lock on graph (no assertion needed)
    }

    // ==================== Export Tests ====================

    use std::sync::Arc;
    use turbovault_tools::ExportTools;

    #[tokio::test]
    async fn test_export_health_report_json() {
        let (_temp, manager) = create_test_vault().await;
        let tools = ExportTools::new(Arc::new(manager));
        let report = tools.export_health_report("json").await.unwrap();

        assert!(report.contains("\"vault_name\""));
        assert!(report.contains("\"health_score\""));
    }

    #[tokio::test]
    async fn test_export_health_report_csv() {
        let (_temp, manager) = create_test_vault().await;
        let tools = ExportTools::new(Arc::new(manager));
        let report = tools.export_health_report("csv").await.unwrap();

        assert!(report.contains("timestamp,vault_name,health_score"));
    }

    #[tokio::test]
    async fn test_export_vault_stats() {
        let (_temp, manager) = create_test_vault().await;
        let tools = ExportTools::new(Arc::new(manager));
        let stats = tools.export_vault_stats("json").await.unwrap();

        assert!(stats.contains("\"total_files\""));
        assert!(stats.contains("\"total_links\""));
    }

    #[tokio::test]
    async fn test_export_analysis_report() {
        let (_temp, manager) = create_test_vault().await;
        let tools = ExportTools::new(Arc::new(manager));
        let report = tools.export_analysis_report("json").await.unwrap();

        assert!(report.contains("\"vault_name\""));
        assert!(report.contains("\"recommendations\""));
    }

    // ==================== Task Metadata Parsing Tests ====================

    /// Tasks.md content covering every metadata format the parser supports:
    /// emoji-only, dataview-only, mixed emoji+dataview, completed tasks,
    /// comma-separated dataview fields, and inline tags.
    const TASKS_MD: &str = "# Test Header
- [ ] Take out the trash 🔁 every day 🛫 2026-04-30 ⏳ 2026-04-30 📅 2026-04-30 🔺
- [ ] Feed the cat ⏫ 🔁 every day 🛫 2026-04-30 ⏳ 2026-04-30 📅 2026-04-30
- [ ] Clean the kitchen 🔁 every week 🛫 2026-04-30 ⏳ 2026-04-30 📅 2026-04-30 #task_type_1
- [ ] Clean the baseboards 🔁 every week 🔽 🛫  2026-04-30 ⏳ 2026-04-30 📅 2026-04-30 #task_type_2
- [ ] Buy groceries [due:: 2026-05-01] [priority:: medium] #errands
- [ ] Write weekly report 📅 2026-05-10 [scheduled:: 2026-05-08] 🔁 every week
- [x] Submit expense report ✅ 2026-04-29 📅 2026-04-28
- [ ] Plan sprint [priority:: high], [start:: 2026-05-01], [due:: 2026-05-07] [id:: sprint-42]
";

    async fn create_task_vault() -> (TempDir, VaultManager) {
        let temp_dir = TempDir::new().expect("temp dir");
        let vault_path = temp_dir.path();

        fs::create_dir_all(vault_path.join("TestFolder"))
            .await
            .expect("create TestFolder");
        fs::write(vault_path.join("TestFolder/Tasks.md"), TASKS_MD)
            .await
            .expect("write Tasks.md");

        let mut config = ConfigProfile::Development.create_config();
        let vault_config = VaultConfig::builder("test", vault_path)
            .build()
            .expect("vault config");
        config.vaults.push(vault_config);

        let manager = VaultManager::new(config).expect("vault manager");
        manager.initialize().await.expect("initialize");

        (temp_dir, manager)
    }

    #[tokio::test]
    async fn test_task_metadata_parsing() {
        let (_temp, manager) = create_task_vault().await;

        let vault_file = manager
            .parse_file(&PathBuf::from("TestFolder/Tasks.md"))
            .await
            .expect("parse Tasks.md");

        assert_eq!(vault_file.tasks.len(), 8, "expected 8 tasks in Tasks.md");

        let find = |desc: &str| {
            vault_file
                .tasks
                .iter()
                .find(|t| t.content == desc)
                .unwrap_or_else(|| panic!("task not found: {desc:?}"))
        };

        // --- Emoji format ---

        let t = find("Take out the trash");
        assert!(!t.is_completed);
        assert_eq!(t.priority, TaskPriority::Highest);
        assert_eq!(t.recurrence.as_deref(), Some("every day"));
        assert_eq!(
            t.start_date.map(|d| d.to_string()).as_deref(),
            Some("2026-04-30")
        );
        assert_eq!(
            t.scheduled_date.map(|d| d.to_string()).as_deref(),
            Some("2026-04-30")
        );
        assert_eq!(
            t.due_date.map(|d| d.to_string()).as_deref(),
            Some("2026-04-30")
        );

        let t = find("Feed the cat");
        assert!(!t.is_completed);
        assert_eq!(t.priority, TaskPriority::High);
        assert_eq!(t.recurrence.as_deref(), Some("every day"));
        assert_eq!(
            t.start_date.map(|d| d.to_string()).as_deref(),
            Some("2026-04-30")
        );
        assert_eq!(
            t.scheduled_date.map(|d| d.to_string()).as_deref(),
            Some("2026-04-30")
        );
        assert_eq!(
            t.due_date.map(|d| d.to_string()).as_deref(),
            Some("2026-04-30")
        );

        let t = find("Clean the kitchen #task_type_1");
        assert_eq!(t.priority, TaskPriority::Normal);
        assert_eq!(t.recurrence.as_deref(), Some("every week"));
        assert_eq!(
            t.start_date.map(|d| d.to_string()).as_deref(),
            Some("2026-04-30")
        );
        assert_eq!(
            t.scheduled_date.map(|d| d.to_string()).as_deref(),
            Some("2026-04-30")
        );
        assert_eq!(
            t.due_date.map(|d| d.to_string()).as_deref(),
            Some("2026-04-30")
        );
        assert_eq!(t.tags, vec!["task_type_1"]);

        // task_type_2 task also tests the double-space after 🛫 (🛫  2026-04-30)
        let t = find("Clean the baseboards #task_type_2");
        assert_eq!(t.priority, TaskPriority::Low);
        assert_eq!(t.recurrence.as_deref(), Some("every week"));
        assert_eq!(
            t.start_date.map(|d| d.to_string()).as_deref(),
            Some("2026-04-30")
        );
        assert_eq!(
            t.scheduled_date.map(|d| d.to_string()).as_deref(),
            Some("2026-04-30")
        );
        assert_eq!(
            t.due_date.map(|d| d.to_string()).as_deref(),
            Some("2026-04-30")
        );
        assert_eq!(t.tags, vec!["task_type_2"]);

        // --- Dataview format ---

        let t = find("Buy groceries #errands");
        assert!(!t.is_completed);
        assert_eq!(t.priority, TaskPriority::Medium);
        assert_eq!(
            t.due_date.map(|d| d.to_string()).as_deref(),
            Some("2026-05-01")
        );
        assert_eq!(t.tags, vec!["errands"]);

        // --- Mixed emoji + Dataview ---

        let t = find("Write weekly report");
        assert!(!t.is_completed);
        assert_eq!(
            t.due_date.map(|d| d.to_string()).as_deref(),
            Some("2026-05-10")
        );
        assert_eq!(
            t.scheduled_date.map(|d| d.to_string()).as_deref(),
            Some("2026-05-08")
        );
        assert_eq!(t.recurrence.as_deref(), Some("every week"));

        // --- Completed task with done date ---

        let t = find("Submit expense report");
        assert!(t.is_completed);
        assert_eq!(
            t.done_date.map(|d| d.to_string()).as_deref(),
            Some("2026-04-29")
        );
        assert_eq!(
            t.due_date.map(|d| d.to_string()).as_deref(),
            Some("2026-04-28")
        );

        // --- Comma-separated Dataview fields with ID ---

        let t = find("Plan sprint");
        assert_eq!(t.priority, TaskPriority::High);
        assert_eq!(
            t.start_date.map(|d| d.to_string()).as_deref(),
            Some("2026-05-01")
        );
        assert_eq!(
            t.due_date.map(|d| d.to_string()).as_deref(),
            Some("2026-05-07")
        );
        assert_eq!(t.id.as_deref(), Some("sprint-42"));
    }

    // ==================== update_task Tests ====================

    /// Helper: replace a specific 1-indexed line in file content and write it back.
    /// Mirrors the logic in the update_task MCP tool.
    async fn apply_task_update(
        manager: &VaultManager,
        file_path: &PathBuf,
        task: &turbovault_core::TaskItem,
    ) {
        let content = manager.read_file(file_path).await.expect("read file");
        let hash = turbovault_vault::compute_hash(&content);

        let line_sep = if content.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let mut lines: Vec<String> = if line_sep == "\r\n" {
            content.split("\r\n").map(|s| s.to_string()).collect()
        } else {
            content.split('\n').map(|s| s.to_string()).collect()
        };

        let line_idx = task.position.line - 1; // 1-indexed → 0-indexed
        let original = &lines[line_idx];
        let indent_len = original.len() - original.trim_start().len();
        let indent = original[..indent_len].to_string();
        lines[line_idx] = format!("{}{}", indent, task.to_markdown_line());

        let new_content = lines.join(line_sep);
        manager
            .write_file(file_path, &new_content, Some(&hash))
            .await
            .expect("write file");
    }

    #[tokio::test]
    async fn test_update_task_mark_complete() {
        use chrono::NaiveDate;
        let (_temp, manager) = create_task_vault().await;
        let file_path = PathBuf::from("TestFolder/Tasks.md");

        // Find the task we want to update
        let vault_file = manager.parse_file(&file_path).await.expect("parse");
        let mut task = vault_file
            .tasks
            .iter()
            .find(|t| t.content == "Take out the trash")
            .expect("find task")
            .clone();

        assert!(!task.is_completed);
        assert!(task.done_date.is_none());

        // Mutate
        task.is_completed = true;
        task.done_date = NaiveDate::from_ymd_opt(2026, 5, 2);

        apply_task_update(&manager, &file_path, &task).await;

        // Re-parse and verify
        let vault_file2 = manager.parse_file(&file_path).await.expect("re-parse");
        let updated = vault_file2
            .tasks
            .iter()
            .find(|t| t.content == "Take out the trash")
            .expect("find updated task");

        assert!(updated.is_completed);
        assert_eq!(
            updated.done_date.map(|d| d.to_string()).as_deref(),
            Some("2026-05-02")
        );
        // Dates from the original task should be preserved
        assert_eq!(
            updated.due_date.map(|d| d.to_string()).as_deref(),
            Some("2026-04-30")
        );
    }

    #[tokio::test]
    async fn test_update_task_change_priority() {
        let (_temp, manager) = create_task_vault().await;
        let file_path = PathBuf::from("TestFolder/Tasks.md");

        let vault_file = manager.parse_file(&file_path).await.expect("parse");
        let mut task = vault_file
            .tasks
            .iter()
            .find(|t| t.content == "Feed the cat")
            .expect("find task")
            .clone();

        assert_eq!(task.priority, TaskPriority::High);

        task.priority = TaskPriority::Lowest;
        apply_task_update(&manager, &file_path, &task).await;

        let vault_file2 = manager.parse_file(&file_path).await.expect("re-parse");
        let updated = vault_file2
            .tasks
            .iter()
            .find(|t| t.content == "Feed the cat")
            .expect("find updated task");

        assert_eq!(updated.priority, TaskPriority::Lowest);
        // Other fields should be unchanged
        assert_eq!(updated.recurrence.as_deref(), Some("every day"));
        assert!(!updated.is_completed);
    }

    #[tokio::test]
    async fn test_update_task_add_and_remove_tags() {
        let (_temp, manager) = create_task_vault().await;
        let file_path = PathBuf::from("TestFolder/Tasks.md");

        let vault_file = manager.parse_file(&file_path).await.expect("parse");
        let mut task = vault_file
            .tasks
            .iter()
            .find(|t| t.content == "Clean the kitchen #task_type_1")
            .expect("find task")
            .clone();

        assert!(task.tags.iter().any(|t| t == "task_type_1"));

        // Add a new tag and remove the existing one
        let bare = "urgent";
        task.content.push(' ');
        task.content.push('#');
        task.content.push_str(bare);
        task.tags.push(bare.to_string());

        // Remove task_type_1
        let remove = "task_type_1";
        let pattern = format!("#{}", remove);
        let lower = task.content.to_lowercase();
        if let Some(pos) = lower.find(&pattern) {
            let end = pos + pattern.len();
            let start = if pos > 0 && task.content.as_bytes().get(pos - 1) == Some(&b' ') {
                pos - 1
            } else {
                pos
            };
            task.content.drain(start..end);
            task.content = task.content.trim_end().to_string();
            task.tags.retain(|t| t != remove);
        }

        apply_task_update(&manager, &file_path, &task).await;

        let vault_file2 = manager.parse_file(&file_path).await.expect("re-parse");
        let updated = vault_file2
            .tasks
            .iter()
            .find(|t| t.tags.iter().any(|tag| tag == "urgent"))
            .expect("find updated task");

        assert!(updated.tags.iter().any(|t| t == "urgent"));
        assert!(!updated.tags.iter().any(|t| t == "task_type_1"));
    }

    #[tokio::test]
    async fn test_update_task_dataview_format_preserved() {
        use chrono::NaiveDate;
        use turbovault_core::TaskMetadataFormat;

        let (_temp, manager) = create_task_vault().await;
        let file_path = PathBuf::from("TestFolder/Tasks.md");

        let vault_file = manager.parse_file(&file_path).await.expect("parse");
        let mut task = vault_file
            .tasks
            .iter()
            .find(|t| t.content == "Buy groceries #errands")
            .expect("find task")
            .clone();

        // Confirm parser detected dataview format
        assert_eq!(task.metadata_format, TaskMetadataFormat::Dataview);

        // Update the due date
        task.due_date = NaiveDate::from_ymd_opt(2026, 6, 1);

        apply_task_update(&manager, &file_path, &task).await;

        // Read raw content to confirm dataview notation was used
        let new_content = manager.read_file(&file_path).await.expect("read");
        assert!(
            new_content.contains("[due:: 2026-06-01]"),
            "expected dataview [due:: ...] notation in: {new_content}"
        );
        assert!(
            !new_content.contains("📅 2026-06-01"),
            "unexpected emoji notation in dataview task"
        );
    }

    #[tokio::test]
    async fn test_update_task_clear_due_date() {
        let (_temp, manager) = create_task_vault().await;
        let file_path = PathBuf::from("TestFolder/Tasks.md");

        let vault_file = manager.parse_file(&file_path).await.expect("parse");
        let mut task = vault_file
            .tasks
            .iter()
            .find(|t| t.content == "Take out the trash")
            .expect("find task")
            .clone();

        assert!(task.due_date.is_some());

        task.due_date = None;
        apply_task_update(&manager, &file_path, &task).await;

        let vault_file2 = manager.parse_file(&file_path).await.expect("re-parse");
        let updated = vault_file2
            .tasks
            .iter()
            .find(|t| t.content == "Take out the trash")
            .expect("find updated task");

        assert!(
            updated.due_date.is_none(),
            "due_date should have been cleared"
        );
    }

    #[tokio::test]
    async fn test_update_task_hash_guard_rejects_stale_write() {
        let (_temp, manager) = create_task_vault().await;
        let file_path = PathBuf::from("TestFolder/Tasks.md");

        // Read the file and capture hash
        let content = manager.read_file(&file_path).await.expect("read");
        let stale_hash = turbovault_vault::compute_hash(&content);

        // Modify the file so the hash becomes stale
        let modified = format!("{}\n<!-- touched -->", content);
        manager
            .write_file(&file_path, &modified, None)
            .await
            .expect("first write");

        // Attempt to write with the now-stale hash — should fail
        let result = manager
            .write_file(&file_path, &modified, Some(&stale_hash))
            .await;

        assert!(result.is_err(), "write with stale hash should fail");
    }

    #[tokio::test]
    async fn test_list_tasks_mcp_tool() {
        let (_temp, manager) = create_task_vault().await;

        let files = manager.scan_vault().await.expect("scan vault");
        let mut all_tasks: Vec<serde_json::Value> = Vec::new();

        for file_path in files {
            if !file_path.to_string_lossy().ends_with(".md") {
                continue;
            }
            let vault_file = manager.parse_file(&file_path).await.expect("parse file");
            for task in vault_file.tasks {
                all_tasks.push(serde_json::json!({
                    "content": task.content,
                    "is_completed": task.is_completed,
                    "path": file_path.to_string_lossy().to_string(),
                    "priority": task.priority,
                    "tags": task.tags,
                }));
            }
        }

        assert_eq!(all_tasks.len(), 8, "expected 8 tasks");

        let find_task = |content: &str| {
            all_tasks
                .iter()
                .find(|t| t["content"] == content)
                .expect(&format!("task not found: {content}"))
        };

        let pending_task = find_task("Take out the trash");
        assert_eq!(pending_task["is_completed"], false);

        let completed_task = find_task("Submit expense report");
        assert_eq!(completed_task["is_completed"], true);

        let tag_task = find_task("Clean the kitchen #task_type_1");
        let tags = tag_task["tags"].as_array().expect("tags array");
        assert!(tags.iter().any(|t| t == "task_type_1"));
    }
}
