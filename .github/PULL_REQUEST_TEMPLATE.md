## Summary
<!-- Concise explanation of root cause / design and touched modules -->

## Linked Issues
<!-- Use `Closes #<id>` / `Fixes #<id>`. `issue-pr-lifecycle.yml` tracks stage labels across `dev -> preview -> main` and guarantees closure on `main` promotion. -->
- Closes #

## Changes by Module
-

## Verification & Invariant Checklist
- [ ] **MSRV 1.70.0 & Zero Crates**: `cargo +1.70.0 check --locked --all-targets && cargo +1.70.0 test --locked`
- [ ] **Formatting & Strict Clippy**: `cargo +stable fmt --all -- --check && cargo +stable clippy --all-targets -- -D warnings`
- [ ] **Isolated Python & Shell Suite**: `python3 -I -m unittest discover -s tests -v`
- [ ] **Modular File Budget**: New/refactored modules stay focused and `< 300` lines (`python3 -I scripts/agent-task-context.py --check-line-budget --changed-only`)
- [ ] **Idempotency & Rollback**: Installer / preference / patcher changes preserve backups and clean `--restore` / `--uninstall`

<!-- ccide-automation: {"schema": 1, "stage": "dev"} -->
