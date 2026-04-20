#![allow(dead_code)]

use crate::core::state::{AccountRankInput, AccountRecord, LiveIdentity, State, UsageSnapshot};

pub fn choose_best_account<'a>(state: &'a State) -> Option<&'a AccountRecord> {
    let mut candidates: Vec<((i64, i64, i64, i64), &AccountRecord)> = state
        .accounts
        .iter()
        .filter_map(|account| {
            let usage = state.usage_cache.get(&account.id)?;
            let rank = usable_rank_input(usage)?;
            Some((build_score(account, usage, rank), account))
        })
        .collect();

    candidates.sort_by(|left, right| right.0.cmp(&left.0));
    candidates.first().map(|(_, account)| *account)
}

pub fn choose_current_account<'a>(
    state: &'a State,
    live: Option<&LiveIdentity>,
) -> Option<&'a AccountRecord> {
    let live = live?;
    state.accounts.iter().find(|account| identity_matches(account, live))
}

pub fn identity_matches(account: &AccountRecord, live: &LiveIdentity) -> bool {
    if !live.adapter_id.is_empty()
        && !account.adapter_id.is_empty()
        && live.adapter_id != account.adapter_id
    {
        return false;
    }

    if live.scodex_account_id.as_deref() == Some(account.id.as_str()) {
        return true;
    }

    if live.stable_id.as_deref() == Some(account.id.as_str()) {
        return true;
    }

    if !live.email.is_empty() && account.effective_display_key().eq_ignore_ascii_case(&live.email) {
        return true;
    }

    if let Some(stable_id) = live.stable_id.as_deref() {
        if account.account_id.as_deref() == Some(stable_id) {
            return true;
        }
        if account.display_key.eq_ignore_ascii_case(stable_id) {
            return true;
        }
    }

    live.aliases.iter().any(|alias| {
        account.effective_display_key().eq_ignore_ascii_case(alias)
            || account.display_key.eq_ignore_ascii_case(alias)
            || account.account_id.as_deref() == Some(alias.as_str())
    })
}

pub fn is_ranked_account_usable(usage: &UsageSnapshot) -> bool {
    usable_rank_input(usage).is_some()
}

fn usable_rank_input(usage: &UsageSnapshot) -> Option<&AccountRankInput> {
    if usage.needs_relogin || usage.last_sync_error.is_some() {
        return None;
    }
    let rank = usage.rank_input.as_ref()?;
    (rank.keep_current_priority > 0 || rank.selection_priority > 0).then_some(rank)
}

fn build_score(
    account: &AccountRecord,
    usage: &UsageSnapshot,
    rank: &AccountRankInput,
) -> (i64, i64, i64, i64) {
    (
        rank.selection_priority,
        rank.freshness_priority,
        usage.last_synced_at.unwrap_or(0),
        account.updated_at,
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{choose_best_account, choose_current_account, identity_matches, is_ranked_account_usable};
    use crate::core::state::{AccountRankInput, AccountRecord, LiveIdentity, State, UsageSnapshot};

    #[test]
    fn choose_best_account_uses_rank_input() {
        let state = State {
            version: 1,
            accounts: vec![
                AccountRecord {
                    id: "weaker".into(),
                    adapter_id: "codex".into(),
                    display_key: "weaker@example.com".into(),
                    email: "weaker@example.com".into(),
                    updated_at: 1,
                    ..AccountRecord::default()
                },
                AccountRecord {
                    id: "stronger".into(),
                    adapter_id: "codex".into(),
                    display_key: "stronger@example.com".into(),
                    email: "stronger@example.com".into(),
                    updated_at: 1,
                    ..AccountRecord::default()
                },
            ],
            usage_cache: BTreeMap::from([
                (
                    "weaker".into(),
                    UsageSnapshot {
                        rank_input: Some(AccountRankInput {
                            keep_current_priority: 1,
                            selection_priority: 10,
                            freshness_priority: 1,
                        }),
                        ..UsageSnapshot::default()
                    },
                ),
                (
                    "stronger".into(),
                    UsageSnapshot {
                        rank_input: Some(AccountRankInput {
                            keep_current_priority: 1,
                            selection_priority: 20,
                            freshness_priority: 1,
                        }),
                        ..UsageSnapshot::default()
                    },
                ),
            ]),
            repo_sync: Default::default(),
        };

        assert_eq!(
            choose_best_account(&state).map(|item| item.id.as_str()),
            Some("stronger")
        );
    }

    #[test]
    fn choose_current_account_matches_adapter_scoped_identity() {
        let state = State {
            version: 1,
            accounts: vec![AccountRecord {
                id: "acct-1".into(),
                adapter_id: "codex".into(),
                display_key: "user@example.com".into(),
                email: "user@example.com".into(),
                account_id: Some("acct-remote-1".into()),
                ..AccountRecord::default()
            }],
            usage_cache: BTreeMap::new(),
            repo_sync: Default::default(),
        };

        let live = LiveIdentity {
            adapter_id: "codex".into(),
            email: "user@example.com".into(),
            account_id: Some("acct-remote-1".into()),
            stable_id: Some("acct-remote-1".into()),
            aliases: Vec::new(),
            payload: serde_json::Value::Null,
            scodex_account_id: None,
        };

        assert_eq!(
            choose_current_account(&state, Some(&live)).map(|item| item.id.as_str()),
            Some("acct-1")
        );
    }

    #[test]
    fn identity_match_rejects_different_adapter_ids() {
        let account = AccountRecord {
            id: "acct-1".into(),
            adapter_id: "codex".into(),
            display_key: "user@example.com".into(),
            email: "user@example.com".into(),
            ..AccountRecord::default()
        };
        let live = LiveIdentity {
            adapter_id: "claude".into(),
            email: "user@example.com".into(),
            account_id: None,
            stable_id: None,
            aliases: Vec::new(),
            payload: serde_json::Value::Null,
            scodex_account_id: None,
        };

        assert!(!identity_matches(&account, &live));
    }

    #[test]
    fn ranked_account_requires_rank_input_and_no_errors() {
        assert!(is_ranked_account_usable(&UsageSnapshot {
            rank_input: Some(AccountRankInput {
                keep_current_priority: 1,
                selection_priority: 1,
                freshness_priority: 1,
            }),
            ..UsageSnapshot::default()
        }));
        assert!(!is_ranked_account_usable(&UsageSnapshot {
            last_sync_error: Some("failed".into()),
            rank_input: Some(AccountRankInput {
                keep_current_priority: 1,
                selection_priority: 1,
                freshness_priority: 1,
            }),
            ..UsageSnapshot::default()
        }));
    }
}
