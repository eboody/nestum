# Agents.md

## Workspace intent
This repo hosts the `nestum` proc-macro crate that generates a shadow module hierarchy for nested enum paths like `Enum1::Variant1::VariantA`.

## Conventions
- Keep the public API minimal and explicit.
- Prefer compile-time errors with clear spans when input is invalid.
- Avoid name collisions by using a dedicated internal module and `pub use` re-exports.
- Add trybuild tests for compile-fail and UI checks.

## Files
- `plan.md`: high-level plan and decisions.
- `nestum/`: proc-macro crate.
