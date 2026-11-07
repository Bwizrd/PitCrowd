**What this changes and why**

**How it was tested**

- [ ] `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test --all-targets` pass in each crate touched
- [ ] Still runs offline with no key (`./start-floor.sh` on a fresh clone)
- [ ] No secrets, no paid calls in tests, no real orders (see [CONTRIBUTING.md](../CONTRIBUTING.md))
