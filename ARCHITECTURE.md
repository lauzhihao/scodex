# Architecture

This repository now uses a Rust core with per-CLI adapters.

## Goals

- Keep the current account-selection behavior and local-first workflow.
- Ship a single cross-platform binary for the wrapper itself.
- Support multiple AI CLIs without forcing them into one fake auth model.
- Let each CLI declare the capabilities it actually supports.

## Non-goals

- Do not assume every CLI supports live usage refresh.
- Do not assume every CLI can switch accounts by replacing one credentials file.

## Layers

### Core

The core owns behavior that should be identical across CLIs:

- command parsing
- state storage
- account records and usage snapshots
- account ranking
- "keep current account if still usable" policy
- shared output formatting

The core must not know about `~/.codex/auth.json`, `~/.claude/.credentials.json`, or any other CLI-specific paths.

### Adapter

Each CLI gets its own adapter. Adapters are responsible for translating the core's generic actions into tool-specific behavior.

Examples:

- discover the active identity for the CLI
- import known credentials into local state
- refresh live usage, if the CLI exposes a reliable source
- switch account, profile, or provider
- run login
- launch or resume the underlying CLI

## Capability model

Each adapter must explicitly declare which features it supports.

- `import_known`
- `read_current_identity`
- `switch_account`
- `login`
- `launch`
- `resume`
- `live_usage`

If an adapter does not support `live_usage`, the core must degrade gracefully instead of pretending automatic account scoring works.

## Current rollout

Phase 1 is complete: Codex support now runs on the Rust implementation.

- keep the Codex path stable in Rust
- preserve local-state compatibility for existing users
- continue tightening tests around install, update, deploy, and account selection flows

Phase 2 adds new adapters one by one.

- `OpenCodeAdapter` is the first candidate after Codex because its auth/config surface is comparatively explicit
- `ClaudeCodeAdapter` and `GeminiCliAdapter` should only move past proof-of-concept after identity switching and usage semantics are validated

The current repository now also includes a stricter target-state blueprint for
the next refactor phase:

- `ADAPTER_FRAMEWORK_V2.md`

That document defines how this codebase should evolve into a reusable
`s-core` style framework plus per-tool adapter crates.

## Repository mapping

- installer and shell integration live in `install.sh` and `install.ps1`
- core policy lives under `src/core`
- CLI-specific Codex behavior lives in `src/adapters/codex.rs`
- top-level command parsing and help live in `src/cli.rs`
