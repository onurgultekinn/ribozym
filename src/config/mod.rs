//! Config loading (§18). Precedence is handled by the caller; this module only
//! parses the three TOML files into typed structs. Every field defaults so a
//! partial config file never crashes the control plane.

pub mod agents;
pub mod models;
pub mod project;

use std::path::Path;

use crate::error::{Error, Result};

/// Repo root is the directory containing `ribozym.toml`; search upward from `start`.
pub fn find_repo_root(start: &Path) -> Option<std::path::PathBuf> {
    let mut dir = Some(start.to_path_buf());
    while let Some(d) = dir {
        if d.join("ribozym.toml").is_file() {
            return Some(d);
        }
        dir = d.parent().map(|p| p.to_path_buf());
    }
    None
}

pub fn load_project_config(root: &Path) -> Result<project::ProjectConfig> {
    let raw = std::fs::read_to_string(root.join("ribozym.toml"))?;
    Ok(toml::from_str(&raw)?)
}

pub fn load_agents(root: &Path) -> Result<Vec<agents::AgentEntry>> {
    let raw = std::fs::read_to_string(root.join("config").join("agents.toml"))?;
    let parsed: agents::AgentsFile = toml::from_str(&raw)?;
    Ok(parsed.agent)
}

/// Load an agent's locked prompt file by its registry name (§11.7:
/// `prompt_file`). Resolution is anchored at `start` (any path inside the
/// repo). Unknown agent or missing file is an error — never an empty prefix
/// (a silent empty system prompt would make the role frame a lie).
pub fn agents_prompt_from(start: &Path, name: &str) -> Result<String> {
    let root = find_repo_root(start)
        .ok_or_else(|| Error::ConfigInvalid("repo root not found for prompts".into()))?;
    let agents = load_agents(&root)?;
    let file = agents
        .iter()
        .find(|a| a.prompt_file.ends_with(&format!("{name}.md")))
        .map(|a| a.prompt_file.clone())
        .ok_or_else(|| Error::ConfigInvalid(format!("no agent prompt named {name:?}")))?;
    std::fs::read_to_string(root.join(&file))
        .map_err(|e| Error::ConfigInvalid(format!("prompt file {file:?} unreadable: {e}")))
}

pub fn load_models(root: &Path) -> Result<models::ModelsFile> {
    let raw = std::fs::read_to_string(root.join("config").join("models.toml"))?;
    Ok(toml::from_str(&raw)?)
}
