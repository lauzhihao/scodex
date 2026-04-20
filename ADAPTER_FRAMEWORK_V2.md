# Adapter Framework V2 Design

## Status

This document defines the next-stage target architecture for turning the current
`scodex` codebase into a reusable adapter framework.

It is intentionally stricter than the current implementation. If the code and
this document diverge, treat this document as the target state for the next
refactor phase rather than as a description of current runtime behavior.

## Problem Statement

The current refactor moved several shared workflows into `core`, but the
boundaries are still `Codex-shaped`:

- `core::state` still models Codex/OpenAI-specific account fields
- `core::policy` still assumes `subscription` vs `api`
- `cli.rs` still contains adapter-specific leakage
- `core::ui` still contains Codex-specific copy
- `AppAdapter` is command-oriented instead of capability-oriented

This is sufficient for a single-repo Codex refactor, but it is not sufficient
for packaging the framework as a reusable dependency for `sclaude` and
`sgemini`.

## Target Outcome

The repository must evolve into three layers:

1. `s-core`
   A reusable Rust library that owns orchestration, generic state, generic sync,
   generic storage, and capability contracts. It must not depend on any
   concrete adapter implementation.
2. `s-codex`
   A Codex-specific library that implements the `s-core` capability
   interfaces.
3. `scodex`
   A thin binary crate that wires the Codex adapter into the generic command
   runner.

Future sibling binaries such as `sclaude` and `sgemini` should follow the same
pattern:

- depend on `s-core`
- provide their own adapter crate
- provide their own thin binary crate

## Design Principles

### 1. Core Owns Flows, Not Product Semantics

`core` may orchestrate:

- account import pipelines
- account selection pipelines
- state persistence
- sync bundle transport
- generic command sequencing

`core` must not understand:

- `Codex`
- `Claude`
- `Gemini`
- `OpenAI`
- `subscription`
- `api`
- `auth.json`
- tool-specific config filenames
- tool-specific quota semantics

### 2. Adapters Own Meaning

An adapter owns:

- how identities are discovered
- how credentials are imported
- how accounts are switched
- which login methods exist
- how usage is refreshed
- how usage is scored
- how adapter-specific fields are stored and restored
- how output should describe adapter-specific entities

### 3. Capabilities Must Be Explicit

The framework must not assume every adapter supports:

- switching by file replacement
- device login
- OAuth login
- API-key accounts
- live usage refresh
- resumable sessions

Every optional behavior must be represented as an explicit capability.

### 4. State Must Use an Envelope Model

The generic state model must preserve core-level indexing and timestamps while
allowing each adapter to store its own payload without polluting the core
schema.

## Target Crate Layout

The logical target layout is:

```text
s-core/
  src/
    command/
    engine/
    policy/
    state/
    storage/
    sync/
    ui/
    adapter/

s-codex/
  src/
    lib.rs
    account.rs
    auth.rs
    launch.rs
    sync.rs
    usage.rs
    ui.rs

scodex/
  src/
    main.rs
```

During the transition, this repository may keep a monorepo layout, but code
must be organized so that the split can happen without semantic rewrites.

## Capability Model

The current `AppAdapter` trait is too command-oriented. The replacement should
be a smaller set of capability interfaces.

Suggested capability split:

### Adapter Metadata

Provides static identity for the adapter.

- adapter id
- display name
- feature flags

### Identity Capability

Responsible for reading the currently active identity from the underlying tool.

- read current identity
- compare a stored account with the live identity

### Account Import Capability

Responsible for discovering and importing credentials into local framework
state.

- import explicit path
- import known locations
- import login result from temporary workspace

### Account Switch Capability

Responsible for making one stored account active in the underlying tool.

- activate account
- optionally clean up or rewrite auxiliary config

### Login Capability

Responsible for adapter-specific authentication workflows.

- default login
- API login if supported
- OAuth login if supported
- any tool-specific login method

### Usage Capability

Responsible for adapter-defined usage refresh and scoring input.

- refresh one account
- indicate whether a stored account can be ranked
- emit normalized rank input for core

### Launch Capability

Responsible for spawning the underlying tool.

- launch
- resume if supported
- passthrough

### Sync Material Capability

Responsible for mapping local adapter account state to a sync-safe bundle
payload and restoring from that payload.

- export account payload
- import account payload

### Presentation Capability

Responsible for adapter-defined table rendering and adapter-specific wording.

- render account table
- adapter-specific command help fragments or message catalog hooks

## Generic State Model

The current `AccountRecord` shape is too specific. The next model should split
generic fields from adapter payload.

Suggested envelope:

```text
ManagedAccountRecord {
  id: String,
  adapter_id: String,
  display_key: String,
  kind: String,
  added_at: i64,
  updated_at: i64,
  payload_version: u32,
  payload: serde_json::Value,
}
```

Suggested companion structures:

```text
ManagedUsageSnapshot {
  last_synced_at: Option<i64>,
  last_sync_error: Option<String>,
  needs_relogin: bool,
  rank_input: Option<AccountRankInput>,
  payload: serde_json::Value,
}

AccountRankInput {
  keep_current_priority: i64,
  selection_priority: i64,
  freshness_priority: i64,
}

LiveIdentity {
  adapter_id: String,
  stable_id: Option<String>,
  aliases: Vec<String>,
  payload: serde_json::Value,
}
```

### Core Rules

`core` may rely on:

- `id`
- `adapter_id`
- `display_key`
- timestamps
- generic rank input
- sync configuration

`core` must not rely on payload internals.

### Adapter Rules

The adapter owns the payload schema and its migrations.

For example, Codex may still internally store:

- email
- account_id
- plan
- auth path
- config path
- provider
- base URL

But these must move into adapter payload rather than stay as core fields.

## Policy Model

`core::policy` must stop hard-coding account kinds such as `subscription` and
`api`.

Instead, `core` should only work with:

- live identity matching result
- `rank_input`
- `needs_relogin`
- `last_sync_error`

Proposed selection flow:

1. ask adapter whether the live identity maps to a managed account
2. if the current managed account is still reusable, keep it
3. otherwise refresh rankable accounts
4. choose the best rankable account from adapter-provided rank input
5. if none exists, optionally run adapter default login

This makes `core` the owner of the sequence while keeping ranking semantics in
adapter-owned data.

## Command Ownership

The command layer should be divided as follows.

### Core-owned Commands

These commands should stay generic:

- `launch`
- `auto`
- `use`
- `rm`
- `list`
- `refresh`
- `push`
- `pull`
- `update`
- `import-auth`
- `import-known`

Core owns the sequence and persistence around these commands.

### Adapter-owned Sub-behaviors

For the commands above, adapters still own:

- account matching
- import semantics
- switch semantics
- usage semantics
- rendering
- launch behavior
- payload export/import

### Adapter-defined Login Surface

`login` and `add` should keep generic top-level names, but the argument model
must become adapter-defined.

The current problem is that `cli.rs` exposes Codex-specific login options in the
generic binary interface. The target state is:

- core defines the command shell
- adapter contributes the argument schema for `login` and `add`
- core passes parsed adapter arguments back to the adapter

If full adapter-defined clap integration is too invasive in one step, use an
intermediate generic request model, but the end state must avoid hard-coding
Codex login options in core.

## UI Ownership

`core::ui` should only contain tool-neutral messages, such as:

- no account available
- invalid sync repo
- update complete
- import failed

Anything that mentions a concrete CLI product or concrete login command must be
owned by the adapter.

Examples that must leave `core::ui`:

- `codex login --device-auth`
- `Starting a fresh Codex session`
- `Deploying the current Codex credential`

## Sync Bundle Format

`core::sync::git` and `core::sync::ssh` are already close to the desired
boundary, but the git bundle currently serializes the current account schema.

The target sync bundle should be envelope-based:

```text
RepoBundleAccount {
  id: String,
  adapter_id: String,
  display_key: String,
  kind: String,
  added_at: i64,
  updated_at: i64,
  payload_version: u32,
  account_payload: serde_json::Value,
}
```

The core sync layer owns encryption, git transport, atomic overwrite, and
bundle validation.

The adapter owns payload serialization and restoration.

## Migration Strategy

This refactor should be executed in controlled phases.

### Phase A: Framework Contracts

- add the new capability-oriented trait design
- define the generic state envelope
- define adapter payload serialization contracts
- keep the current Codex adapter behavior unchanged

### Phase B: Dual-Path Codex Migration

- adapt Codex to the new state envelope
- adapt Codex ranking to emit generic rank input
- move Codex-specific messages out of `core::ui`
- remove CLI references to `crate::adapters::codex::*`

### Phase C: Core/Binary Separation

- isolate reusable modules into `s-core`
- move Codex logic into `s-codex`
- keep the current binary as a thin assembly crate

### Phase D: Framework Validation

- add a minimal `dummy` adapter used only for tests
- prove that the core can operate without Codex-specific assumptions

### Phase E: External Adapter Adoption

- implement `sclaude`
- implement `sgemini`

The `dummy` adapter is important because it validates framework boundaries
without importing a second real product's complexity too early.

## Transitional Constraints

During migration:

- behavior regressions on existing Codex flows are not acceptable
- state compatibility should be preserved where practical
- every interface move should be covered by `cargo test`
- command-level verification should continue using `scripts/cli-regression.sh`

## Immediate Next Refactor Tasks

The next implementation batch should focus on these items first:

1. remove direct Codex references from `cli.rs`
2. define a generic account envelope in `core::state`
3. define generic rank input and adapter-owned usage payload
4. redesign `AppAdapter` into capability-oriented contracts
5. move Codex-specific UI copy out of `core::ui`
6. adapt sync bundle serialization to the generic envelope

Only after these are complete should the repository be split into multiple
publishable crates.
