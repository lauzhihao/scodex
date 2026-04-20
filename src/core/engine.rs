use std::path::Path;

use anyhow::Result;

use crate::adapters::AppAdapter;
use crate::core::policy::{choose_best_account, choose_current_account, choose_current_api_account};
use crate::core::state::{AccountRecord, State, UsageSnapshot};

pub fn ensure_best_account<A: AppAdapter>(
    adapter: &A,
    state_dir: &Path,
    state: &mut State,
    no_import_known: bool,
    no_login: bool,
    perform_switch: bool,
) -> Result<Option<(AccountRecord, UsageSnapshot)>> {
    if !no_import_known {
        adapter.import_known_sources(state_dir, state);
    }

    if state.accounts.is_empty() {
        if no_login {
            return Ok(None);
        }
        let record = adapter.login_default(state_dir, state)?;
        let usage = adapter.refresh_usage(state, &record);
        if perform_switch {
            adapter.switch_account(&record)?;
        }
        return Ok(Some((record, usage)));
    }

    let live_identity = adapter.read_live_identity();
    if let Some(current) = choose_current_api_account(state, live_identity.as_ref()).cloned() {
        let usage = UsageSnapshot::default();
        if perform_switch {
            adapter.switch_account(&current)?;
        }
        return Ok(Some((current, usage)));
    }

    refresh_all_accounts(adapter, state);
    if let Some(current) = choose_current_account(state, live_identity.as_ref()).cloned() {
        let usage = state.usage_cache.get(&current.id).cloned().unwrap_or_default();
        if perform_switch {
            adapter.switch_account(&current)?;
        }
        return Ok(Some((current, usage)));
    }

    if let Some(best) = choose_best_account(state).cloned() {
        let usage = state.usage_cache.get(&best.id).cloned().unwrap_or_default();
        if perform_switch {
            adapter.switch_account(&best)?;
        }
        return Ok(Some((best, usage)));
    }

    if no_login {
        return Ok(None);
    }
    let record = adapter.login_default(state_dir, state)?;
    let usage = adapter.refresh_usage(state, &record);
    if perform_switch {
        adapter.switch_account(&record)?;
    }
    Ok(Some((record, usage)))
}

pub fn refresh_all_accounts<A: AppAdapter>(adapter: &A, state: &mut State) {
    let accounts = state.accounts.clone();
    for account in accounts {
        let _ = adapter.refresh_usage(state, &account);
    }
}

pub fn find_account_by_email<'a>(state: &'a State, email: &str) -> Option<&'a AccountRecord> {
    state
        .accounts
        .iter()
        .find(|account| account.email.eq_ignore_ascii_case(email))
}
