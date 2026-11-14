## What and why

<!-- One or two sentences. Link the issue or the plan this belongs to. -->

## Checks

- [ ] `yarn typecheck` and `yarn test` pass
- [ ] `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` pass (from `src-tauri/`)
- [ ] Tests came first for new logic (see CONTRIBUTING.md)
- [ ] Dependencies changed: I ran `yarn run licenses`
- [ ] Every commit is signed off (`git commit -s`), see the DCO in CONTRIBUTING.md
- [ ] Nothing here touches a user's originals, and no personal data is in the diff
