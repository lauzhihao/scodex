#![allow(dead_code)]

use std::ffi::OsString;
use std::path::Path;

use anyhow::Result;

use crate::core::state::{AccountRecord, LiveIdentity, State, UsageSnapshot};

pub mod codex;

#[derive(Debug, Clone, Copy)]
pub struct AdapterCapabilities {
    pub import_known: bool,
    pub read_current_identity: bool,
    pub switch_account: bool,
    pub login: bool,
    pub launch: bool,
    pub resume: bool,
    pub live_usage: bool,
}

pub trait CliAdapter {
    fn id(&self) -> &'static str;
    fn capabilities(&self) -> AdapterCapabilities;
}

pub trait AppAdapter {
    fn display_name(&self) -> &'static str;

    fn normalize_account_records(&self, state: &mut State) -> bool;
    fn handle_login(
        &self,
        state_dir: &Path,
        state: &mut State,
        args: &crate::cli::LoginArgs,
    ) -> Result<AccountRecord>;
    fn login_default(&self, state_dir: &Path, state: &mut State) -> Result<AccountRecord>;
    fn handle_add(
        &self,
        state_dir: &Path,
        state: &mut State,
        args: &crate::cli::AddArgs,
    ) -> Result<AccountRecord>;
    fn import_known_sources(&self, state_dir: &Path, state: &mut State) -> Vec<AccountRecord>;
    fn find_account_by_email<'a>(
        &self,
        state: &'a State,
        email: &str,
    ) -> Option<&'a AccountRecord>;
    fn switch_account(&self, record: &AccountRecord) -> Result<()>;
    fn remove_account(&self, state_dir: &Path, state: &mut State, id: &str) -> Result<()>;
    fn handle_deploy(&self, target: &str, identity_file: Option<&Path>) -> Result<()>;
    fn handle_push(
        &self,
        state: &State,
        repo: &str,
        path: Option<&str>,
        identity_file: Option<&Path>,
    ) -> Result<crate::core::sync::git::PushOutcome>;
    fn handle_pull(
        &self,
        state_dir: &Path,
        state: &mut State,
        repo: &str,
        path: Option<&str>,
        identity_file: Option<&Path>,
    ) -> Result<crate::core::sync::git::PullOutcome>;
    fn read_live_identity(&self) -> Option<LiveIdentity>;
    fn render_account_table(&self, state: &State, active: Option<&LiveIdentity>) -> String;
    fn handle_import_auth(
        &self,
        state_dir: &Path,
        state: &mut State,
        path: &Path,
    ) -> Result<AccountRecord>;
    fn refresh_usage(&self, state: &mut State, record: &AccountRecord) -> UsageSnapshot;
    fn launch_process(&self, extra_args: &[OsString], resume: bool) -> Result<i32>;
    fn run_passthrough(&self, extra_args: &[OsString]) -> Result<i32>;
}
