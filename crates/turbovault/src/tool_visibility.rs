use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    future::Future,
    path::{Path, PathBuf},
};
use turbomcp::{
    McpError, McpHandler, McpResult, Prompt, PromptResult, RequestContext, Resource, ResourceResult,
    ServerInfo, Tool, ToolResult,
};
use turbomcp::__macro_support::turbomcp_core::marker::MaybeSend;
use turbomcp_server::alias::AliasConfig;
use turbomcp_server::__macro_support::turbomcp_types::{ListTasksResult, ResourceTemplate, Task};

/// User-facing tool visibility settings loaded from TurboVault config.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolVisibilitySettings {
    /// If non-empty, only these exact tool names are listed and callable.
    pub allowed: Vec<String>,
    /// Exact tool names omitted from `tools/list` but still callable by name.
    pub hidden: Vec<String>,
    /// Exact tool names omitted from `tools/list` and rejected on direct calls.
    pub disabled: Vec<String>,
    /// Hide tools that are not annotated read-only by TurboMCP.
    pub require_read_only: bool,
}

/// CLI/env overrides merged with file-based tool visibility settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolVisibilityOverrides {
    pub allowed: Vec<String>,
    pub hidden: Vec<String>,
    pub disabled: Vec<String>,
    pub require_read_only: bool,
}

/// Full TurboVault server config loaded from the YAML config file.
///
/// Combines tool-visibility rules with tool-alias definitions.
#[derive(Debug, Clone, Default)]
pub struct TurboVaultConfig {
    pub tool_visibility: ToolVisibilitySettings,
    pub tool_aliases: AliasConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct TurboVaultConfigFile {
    tool_visibility: ToolVisibilitySettings,
    tool_aliases: AliasConfig,
}

impl TurboVaultConfig {
    /// Parse both visibility settings and alias config from a YAML string.
    pub fn from_yaml_str(yaml: &str) -> anyhow::Result<Self> {
        let file: TurboVaultConfigFile =
            yaml_serde::from_str(yaml).context("invalid TurboVault YAML config")?;
        Ok(Self {
            tool_visibility: file.tool_visibility,
            tool_aliases: file.tool_aliases,
        })
    }

    /// Load both visibility settings and alias config from a YAML config file.
    pub async fn from_yaml_file(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let yaml = tokio::fs::read_to_string(path)
            .await
            .with_context(|| format!("failed to read config {}", path.display()))?;
        Self::from_yaml_str(&yaml)
            .with_context(|| format!("failed to parse config {}", path.display()))
    }
}

impl ToolVisibilitySettings {
    /// Parse the `tool_visibility` section from a TurboVault YAML config.
    pub fn from_yaml_str(yaml: &str) -> anyhow::Result<Self> {
        let config: TurboVaultConfigFile =
            yaml_serde::from_str(yaml).context("invalid TurboVault YAML config")?;
        Ok(config.tool_visibility)
    }

    /// Load the `tool_visibility` section from a YAML config file.
    pub async fn from_yaml_file(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let yaml = tokio::fs::read_to_string(path)
            .await
            .with_context(|| format!("failed to read tool visibility config {}", path.display()))?;
        Self::from_yaml_str(&yaml)
            .with_context(|| format!("failed to parse tool visibility config {}", path.display()))
    }

    /// Merge CLI/env overrides into file settings.
    pub fn merge_cli(&mut self, overrides: ToolVisibilityOverrides) {
        extend_clean(&mut self.allowed, overrides.allowed);
        extend_clean(&mut self.hidden, overrides.hidden);
        extend_clean(&mut self.disabled, overrides.disabled);
        self.require_read_only |= overrides.require_read_only;
    }

    /// Returns true if any visibility rules are configured.
    pub fn has_rules(&self) -> bool {
        !self.allowed.is_empty()
            || !self.hidden.is_empty()
            || !self.disabled.is_empty()
            || self.require_read_only
    }

    /// Returns true if this tool should appear in `tools/list` (name-based check only).
    ///
    /// Note: `require_read_only` is applied at the [`ToolNameFilter`] layer using the
    /// tool's actual annotation; this method checks only the name-based lists.
    pub fn is_listed(&self, name: &str) -> bool {
        if self.disabled.iter().any(|n| n == name) {
            return false;
        }
        if self.hidden.iter().any(|n| n == name) {
            return false;
        }
        if !self.allowed.is_empty() && !self.allowed.iter().any(|n| n == name) {
            return false;
        }
        true
    }

    /// Returns true if direct calls to this tool are allowed (name-based check only).
    ///
    /// Hidden tools are not listed but ARE callable. Disabled tools (and tools
    /// outside an active allow-list) reject direct calls.
    pub fn is_enabled(&self, name: &str) -> bool {
        if self.disabled.iter().any(|n| n == name) {
            return false;
        }
        if !self.allowed.is_empty() && !self.allowed.iter().any(|n| n == name) {
            return false;
        }
        true
    }
}

/// Wraps an [`McpHandler`] and applies name-based tool visibility rules from
/// [`ToolVisibilitySettings`].
///
/// This is the TurboVault-native replacement for turbomcp's tag-based
/// `VisibilityLayer`, providing exact-name allow/hide/disable semantics
/// as well as read-only enforcement.
///
/// # Composition order with `AliasLayer`
///
/// Always place `ToolNameFilter` on the **outside** of
/// [`AliasLayer`](turbomcp_server::AliasLayer) so that alias tools are subject
/// to the same visibility rules as real tools:
///
/// ```text
/// ToolNameFilter(outer)       <- filters merged tool list
///   AliasLayer(middle)        <- exposes aliases as first-class tools
///     ObsidianMcpServer(inner) <- real tool implementations
/// ```
#[derive(Clone)]
pub struct ToolNameFilter<H> {
    inner: H,
    settings: ToolVisibilitySettings,
}

impl<H> ToolNameFilter<H> {
    pub fn new(inner: H, settings: ToolVisibilitySettings) -> Self {
        Self { inner, settings }
    }

    pub fn inner(&self) -> &H {
        &self.inner
    }

    pub fn into_inner(self) -> H {
        self.inner
    }
}

#[allow(clippy::manual_async_fn)]
impl<H: McpHandler> McpHandler for ToolNameFilter<H> {
    fn server_info(&self) -> ServerInfo {
        self.inner.server_info()
    }

    fn list_tools(&self) -> Vec<Tool> {
        self.inner
            .list_tools()
            .into_iter()
            .filter(|t| {
                if !self.settings.is_listed(&t.name) {
                    return false;
                }
                if self.settings.require_read_only {
                    let is_ro = t
                        .annotations
                        .as_ref()
                        .and_then(|a| a.read_only_hint)
                        .unwrap_or(false);
                    if !is_ro {
                        return false;
                    }
                }
                true
            })
            .collect()
    }

    fn list_resources(&self) -> Vec<Resource> {
        self.inner.list_resources()
    }

    fn list_prompts(&self) -> Vec<Prompt> {
        self.inner.list_prompts()
    }

    fn call_tool<'a>(
        &'a self,
        name: &'a str,
        args: Value,
        ctx: &'a RequestContext,
    ) -> impl Future<Output = McpResult<ToolResult>> + MaybeSend + 'a {
        async move {
            if !self.settings.has_rules() {
                return self.inner.call_tool(name, args, ctx).await;
            }
            // Name-based enforcement: disabled + allow-list.
            if !self.settings.is_enabled(name) {
                return Err(McpError::tool_not_found(name));
            }
            // require_read_only: look up the tool's annotation.
            if self.settings.require_read_only {
                let tools = self.inner.list_tools();
                if let Some(tool) = tools.iter().find(|t| t.name == name) {
                    let is_ro = tool
                        .annotations
                        .as_ref()
                        .and_then(|a| a.read_only_hint)
                        .unwrap_or(false);
                    if !is_ro {
                        return Err(McpError::tool_not_found(name));
                    }
                }
            }
            self.inner.call_tool(name, args, ctx).await
        }
    }

    fn read_resource<'a>(
        &'a self,
        uri: &'a str,
        ctx: &'a RequestContext,
    ) -> impl Future<Output = McpResult<ResourceResult>> + MaybeSend + 'a {
        async move { self.inner.read_resource(uri, ctx).await }
    }

    fn get_prompt<'a>(
        &'a self,
        name: &'a str,
        args: Option<Value>,
        ctx: &'a RequestContext,
    ) -> impl Future<Output = McpResult<PromptResult>> + MaybeSend + 'a {
        async move { self.inner.get_prompt(name, args, ctx).await }
    }

    fn on_initialize(&self) -> impl Future<Output = McpResult<()>> + MaybeSend {
        let fut = self.inner.on_initialize();
        async move { fut.await }
    }

    fn on_shutdown(&self) -> impl Future<Output = McpResult<()>> + MaybeSend {
        let fut = self.inner.on_shutdown();
        async move { fut.await }
    }

    // ===== Delegate all remaining trait methods to the inner handler =====
    //
    // MAINTENANCE NOTE: when McpHandler gains new methods (e.g. a new
    // subscription or task API), add a delegation here. Omitting a method
    // silently falls through to the trait's no-op default and drops any
    // behavior the inner handler provides.

    fn list_resource_templates(&self) -> Vec<ResourceTemplate> {
        self.inner.list_resource_templates()
    }

    fn complete<'a>(
        &'a self,
        params: Value,
        ctx: &'a RequestContext,
    ) -> impl Future<Output = McpResult<Value>> + MaybeSend + 'a {
        async move { self.inner.complete(params, ctx).await }
    }

    fn subscribe<'a>(
        &'a self,
        uri: &'a str,
        ctx: &'a RequestContext,
    ) -> impl Future<Output = McpResult<()>> + MaybeSend + 'a {
        async move { self.inner.subscribe(uri, ctx).await }
    }

    fn unsubscribe<'a>(
        &'a self,
        uri: &'a str,
        ctx: &'a RequestContext,
    ) -> impl Future<Output = McpResult<()>> + MaybeSend + 'a {
        async move { self.inner.unsubscribe(uri, ctx).await }
    }

    fn set_log_level<'a>(
        &'a self,
        level: &'a str,
        ctx: &'a RequestContext,
    ) -> impl Future<Output = McpResult<()>> + MaybeSend + 'a {
        async move { self.inner.set_log_level(level, ctx).await }
    }

    fn list_tasks<'a>(
        &'a self,
        cursor: Option<&'a str>,
        limit: Option<usize>,
        ctx: &'a RequestContext,
    ) -> impl Future<Output = McpResult<ListTasksResult>> + MaybeSend + 'a {
        async move { self.inner.list_tasks(cursor, limit, ctx).await }
    }

    fn get_task<'a>(
        &'a self,
        task_id: &'a str,
        ctx: &'a RequestContext,
    ) -> impl Future<Output = McpResult<Task>> + MaybeSend + 'a {
        async move { self.inner.get_task(task_id, ctx).await }
    }

    fn cancel_task<'a>(
        &'a self,
        task_id: &'a str,
        ctx: &'a RequestContext,
    ) -> impl Future<Output = McpResult<Task>> + MaybeSend + 'a {
        async move { self.inner.cancel_task(task_id, ctx).await }
    }

    fn get_task_result<'a>(
        &'a self,
        task_id: &'a str,
        ctx: &'a RequestContext,
    ) -> impl Future<Output = McpResult<Value>> + MaybeSend + 'a {
        async move { self.inner.get_task_result(task_id, ctx).await }
    }
}

/// Default TurboVault user config path.
pub fn default_config_path() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .map(|home| home.join(".turbovault").join("config.yaml"))
}

fn extend_clean(target: &mut Vec<String>, values: Vec<String>) {
    for value in values {
        let value = value.trim();
        if !value.is_empty() && !target.iter().any(|existing| existing == value) {
            target.push(value.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tool_visibility_from_yaml() {
        let yaml = r#"
tool_visibility:
  allowed:
    - read_note
    - search
    - full_health_analysis
  hidden:
    - full_health_analysis
  disabled:
    - delete_note
  require_read_only: true
"#;

        let settings = ToolVisibilitySettings::from_yaml_str(yaml).unwrap();

        // read_note is in allowed and not hidden/disabled.
        assert!(settings.is_listed("read_note"));
        // full_health_analysis is in allowed but hidden: not listed.
        assert!(!settings.is_listed("full_health_analysis"));
        // hidden tools remain callable.
        assert!(settings.is_enabled("full_health_analysis"));
        // disabled tools are not callable.
        assert!(!settings.is_enabled("delete_note"));
        // require_read_only flag is preserved.
        assert!(settings.require_read_only);
    }

    #[test]
    fn empty_config_keeps_all_tools_visible_and_callable() {
        let settings = ToolVisibilitySettings::from_yaml_str("{}").unwrap();

        assert!(settings.is_listed("read_note"));
        assert!(settings.is_enabled("delete_note"));
        assert!(!settings.require_read_only);
    }

    #[test]
    fn cli_overrides_merge_with_file_settings() {
        let yaml = r#"
tool_visibility:
  hidden:
    - full_health_analysis
  disabled:
    - delete_note
"#;

        let mut settings = ToolVisibilitySettings::from_yaml_str(yaml).unwrap();
        settings.merge_cli(ToolVisibilityOverrides {
            allowed: vec!["read_note".to_string()],
            hidden: vec!["query_frontmatter_sql".to_string()],
            disabled: vec!["write_note".to_string()],
            require_read_only: true,
        });

        // Only read_note is in the allow-list: others are not listed.
        assert!(settings.is_listed("read_note"));
        // delete_note is disabled regardless of allow-list.
        assert!(!settings.is_enabled("delete_note"));
        // write_note was disabled via CLI.
        assert!(!settings.is_enabled("write_note"));
        // full_health_analysis is hidden: not listed but still callable via name.
        assert!(!settings.is_listed("full_health_analysis"));
        // query_frontmatter_sql was hidden via CLI.
        assert!(!settings.is_listed("query_frontmatter_sql"));
        assert!(settings.require_read_only);
    }

    #[test]
    fn parses_tool_aliases_from_yaml() {
        let yaml = r#"
tool_aliases:
  aliases:
    - name: find_journal
      tool: search
      description: Search within Daily Journal notes
      preset_args:
        tag: journal
    - name: list_tasks
      tool: query_metadata
      preset_args:
        key: status
        value: todo
"#;

        let config = TurboVaultConfig::from_yaml_str(yaml).unwrap();
        assert_eq!(config.tool_aliases.aliases.len(), 2);
        assert_eq!(config.tool_aliases.aliases[0].name, "find_journal");
        assert_eq!(config.tool_aliases.aliases[0].tool, "search");
        assert_eq!(
            config.tool_aliases.aliases[0].description.as_deref(),
            Some("Search within Daily Journal notes")
        );
        assert_eq!(
            config.tool_aliases.aliases[0]
                .preset_args
                .get("tag")
                .and_then(|v| v.as_str()),
            Some("journal")
        );
        assert_eq!(config.tool_aliases.aliases[1].name, "list_tasks");
    }

    #[test]
    fn empty_tool_aliases_section_is_valid() {
        let config = TurboVaultConfig::from_yaml_str("{}").unwrap();
        assert!(config.tool_aliases.aliases.is_empty());
    }

    #[test]
    fn combined_config_parses_both_sections() {
        let yaml = r#"
tool_visibility:
  disabled:
    - delete_note
tool_aliases:
  aliases:
    - name: quick_search
      tool: search
"#;
        let config = TurboVaultConfig::from_yaml_str(yaml).unwrap();
        assert!(config.tool_visibility.disabled.contains(&"delete_note".to_string()));
        assert_eq!(config.tool_aliases.aliases.len(), 1);
        assert_eq!(config.tool_aliases.aliases[0].name, "quick_search");
    }
}
