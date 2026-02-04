# Plan: nestum proc-macro (shadow module hierarchy)

## Goal
Provide a proc-macro that lets users write nested paths like `Enum1::Variant1::VariantA` to construct and reference nested enums, without requiring explicit tuple-style nesting like `Enum1::Variant1(Enum2::VariantA)`.

## API decisions (current)
- Use an attribute proc-macro: `#[nestum]` on an enum.
- Nested variants are auto-detected: any tuple variant with exactly one field whose type is a simple enum ident that is also marked with `#[nestum]` in the same module.
- Cross-module nesting is opt-in per variant via `#[nestum(external = "crate::path::Enum")]`, including enums declared in other module files.
- Expansion generates a shadow module named after the enum. The enum type itself becomes `EnumName::EnumName`.
- Inner enums do not need to be annotated; the macro reads module enums from source and keeps a registry.

## Approach (Option A)
Generate a shadow module hierarchy that mirrors the enum + variant names and adds wrapper constructors so `Enum1::Variant1::VariantA` resolves to the *outer* enum value.

## Steps
1) Define macro API and input format
   - `#[nestum]` attribute on enums.
   - Auto-detect nested variants by tuple variant shape + inner enum type marked with `#[nestum]`.
   - Optional `#[nestum(external = \"crate::path::Enum\")]` on variants to reference an enum in a different module.

2) Code generation strategy (implemented)
   - Replace each marked enum item with a module of the same name.
   - Inside the module, emit the enum with its original name and variants.
   - For each nested variant, emit a submodule with wrapper constructors for every inner enum variant.

3) Name/visibility rules
   - Shadow module uses the original enum name; enum type becomes `EnumName::EnumName`.
   - Nested modules use the outer variant name.

4) Registry and source discovery (implemented)
   - Read the current source file to collect enums by module path.
   - Cache enum metadata in a global registry keyed by module path.

5) Tests
   - Add a trybuild pass test covering `Enum1::Variant1::VariantA`.
   - Add compile-fail tests in a follow-up when error output stabilizes.
