# Contributing to NOVA DB

Thank you for your interest in contributing to NOVA DB!

## Development Workflow

1. Fork and clone the repository.
2. Ensure you have a recent stable Rust toolchain (1.80+ recommended).
3. Build the workspace:
   ```bash
   cargo build
   ```
4. Run all unit and integration tests:
   ```bash
   cargo test --workspace
   ```
5. Check code formatting and Clippy lints:
   ```bash
   cargo fmt --check
   cargo clippy --all-targets -- -D warnings
   ```

## Code Quality Standards

- Maintain zero compiler and Clippy warnings.
- Keep modules focused with clear architectural boundaries.
- Write unit tests alongside critical storage, parsing, and networking logic.
- Avoid unnecessary external dependencies.
