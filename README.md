# nestum

`nestum` is a proc-macro that makes *nested enum paths* feel natural, so you can write:

```rust
Enum1::Variant1::VariantA
```

instead of:

```rust
Enum1::Variant1(Enum2::VariantA)
```

It does this by generating a shadow module hierarchy and wrapper constructors around your enums.

## Why
Rust enums are great for modeling state and variants, but nested enum patterns quickly get noisy when you need multiple levels:

```rust
Enum1::Variant1(Enum2::VariantA)
```

`nestum` removes the visual clutter by letting you access nested variants via paths:

```rust
Enum1::Variant1::VariantA
```

You still get the same enum types and matching semantics—you just get cleaner call sites.

## Quick Start

```rust
use nestum::nestum;

#[nestum]
pub enum Enum2 {
    VariantA,
    VariantB(u8),
    VariantC { x: i32 },
}

#[nestum]
pub enum Enum1 {
    Variant1(Enum2),
    Other,
}

fn main() {
    let _ = Enum1::Variant1::VariantA;
    let _ = Enum1::Variant1::VariantB(1);
    let _ = Enum1::Variant1::VariantC(2);
    let _ = Enum1::Enum1::Other;
}
```

## Concepts

### Shadow module
When `#[nestum]` is applied to an enum, the macro replaces it with a module of the same name.
Inside that module, it re-emits the original enum and creates submodules for nested variants.
Those submodules expose wrapper constructors that build the outer enum.

This means the enum type itself is accessed as `EnumName::EnumName`.

### Nested variant detection
A variant is treated as nested **if and only if**:
- it is a tuple variant with exactly one field, and
- the inner field’s type is a simple enum ident that is also marked with `#[nestum]` in the same module.

This keeps nesting explicit without requiring per-variant annotations.

### Cross-module nesting
To nest an enum declared in a different module file, use an external path on the variant:

```rust
use nestum::nestum;

mod inner;

#[nestum]
pub enum Outer {
    #[nestum(external = "crate::inner::Inner")]
    Wrap(Inner),
}

fn main() {
    let _ = Outer::Wrap::A;
}
```

This resolves `crate::inner::Inner` to `src/inner.rs` or `src/inner/mod.rs` and generates wrappers.

## API Summary

### `#[nestum]` on enums
Enables shadow-module generation and wrapper constructors.

### `#[nestum(external = "path::to::Enum")]` on variants
Opt-in support for nesting an enum in another module file.

### `nestum_match! { match value { ... } }`
Macro that rewrites nested patterns (like `Enum1::Variant1::VariantA`) into real enum patterns.

## Examples

### Basic nesting
```rust
#[nestum]
pub enum Inner { A, B(u8) }

#[nestum]
pub enum Outer { Wrap(Inner) }

let _ = Outer::Wrap::A;
let _ = Outer::Wrap::B(1);
```

### Cross-module nesting
```rust
mod inner;

#[nestum]
pub enum Outer {
    #[nestum(external = "crate::inner::Inner")]
    Wrap(Inner),
}

let _ = Outer::Wrap::A;
```

### Nested match patterns
```rust
use nestum::{nestum, nestum_match};

#[nestum]
pub enum Inner { A, B(u8) }

#[nestum]
pub enum Outer { Wrap(Inner), Other }

let value = Outer::Wrap::A;
nestum_match! {
    match value {
        Outer::Wrap::A => {}
        Outer::Wrap::B(n) => { let _ = n; }
        Outer::Outer::Other => {}
    }
}
```

## Limitations
- External crates are not supported (proc macros can’t reliably inspect other crates’ ASTs).
- The macro only resolves enums from source files in the current crate.
- `#[path = "..."]`, `include!()`, and complex `cfg` layouts may not be resolved.

## Error Messages
`nestum` emits detailed compile-time errors, including:
- invalid `#[nestum(...)]` usage,
- misuse on variants,
- external path mismatches,
- missing module files or enums.

## FAQ

### Why does the enum type become `EnumName::EnumName`?
Because `nestum` replaces the enum with a module of the same name. The original enum is re-emitted inside it.

### How do I pattern match with nested paths?
Use `nestum_match!` so paths like `Enum1::Variant1::VariantA` are rewritten to the real enum pattern.

### Why not support external crates?
The macro would need to discover and parse dependency source files, which is brittle and not reliably possible in stable proc-macro APIs.

## License
MIT
