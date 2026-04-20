use std::path::Path;

use anyhow::Result;

use super::CodexAdapter;
use super::paths::codex_home;
use crate::core::sync::ssh;

impl CodexAdapter {
    pub fn deploy_live_auth(&self, target: &str, identity_file: Option<&Path>) -> Result<()> {
        let source = codex_home().join("auth.json");
        ssh::deploy_file(&source, target, "auth.json", identity_file)
    }
}
