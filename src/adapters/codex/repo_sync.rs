use std::path::Path;

use anyhow::Result;

use super::CodexAdapter;
use crate::core::state::State;
use crate::core::sync::git::{self, PullOutcome, PushOutcome};

impl CodexAdapter {
    pub fn push_account_pool(
        &self,
        state: &State,
        repo: &str,
        bundle_dir: Option<&str>,
        identity_file: Option<&Path>,
    ) -> Result<PushOutcome> {
        git::push_account_pool(state, repo, bundle_dir, identity_file, super::now_ts())
    }

    pub fn pull_account_pool(
        &self,
        state_dir: &Path,
        state: &mut State,
        repo: &str,
        bundle_dir: Option<&str>,
        identity_file: Option<&Path>,
    ) -> Result<PullOutcome> {
        git::pull_account_pool(state_dir, state, repo, bundle_dir, identity_file)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use crate::core::sync::git::{DEFAULT_BUNDLE_DIR, resolve_bundle_dir};

    #[test]
    fn bundle_dir_defaults_when_missing() -> Result<()> {
        assert_eq!(
            resolve_bundle_dir(None)?,
            std::path::PathBuf::from(DEFAULT_BUNDLE_DIR)
        );
        Ok(())
    }
}
