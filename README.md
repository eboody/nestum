<div align="center">
  <img alt="nestum logo" src="https://raw.githubusercontent.com/eboody/nestum/main/nestum.png" width="360">
  <p>Nestum keeps the correct nested-enum model and removes the syntax tax.</p>
  <p>
    <a href="https://github.com/eboody/nestum/actions/workflows/ci.yml"><img src="https://github.com/eboody/nestum/actions/workflows/ci.yml/badge.svg?branch=main&event=push" alt="build status" /></a>
    <a href="https://crates.io/crates/nestum"><img src="https://img.shields.io/crates/v/nestum.svg?logo=rust" alt="crates.io" /></a>
    <a href="https://docs.rs/nestum"><img src="https://docs.rs/nestum/badge.svg" alt="docs.rs" /></a>
  </p>
</div>

# Nestum

When nested enums are the honest model, they buy you real compile-time invariants.

If `Event::Document` wraps `DocumentEvent`, then the type system already rules out the wrong family. You cannot accidentally construct a document envelope around a user event. The problem is not correctness. The problem is that the accurate model gets noisy fast:

Construct:

```rust
Event::Document::Created
```

instead of:

```rust
Event::Document(DocumentEvent::Created)
```

and match the same way with `nested!`.

```rust
use nestum::{nestum, nested};

#[nestum]
enum DocumentEvent {
    Created,
    Deleted,
}

#[nestum]
enum Event {
    Document(DocumentEvent),
}

let inner: DocumentEvent::Enum = DocumentEvent::Created;
let event: Event::Enum = Event::Document::Created;

nested! {
    match event {
        Event::Document::Created => {}
        Event::Document::Deleted => {}
    }
}

let _ = inner;
```

`nestum` keeps the nested-enum design, keeps the invariant, and removes most of the wrapping noise.

## Correctness First

Use `nestum` when nested enums are already the right way to model the domain.

- `Event::Document` always contains a `DocumentEvent`.
- `Event::User` always contains a `UserEvent`.
- cross-family mistakes stay impossible at compile time.

`nestum` does not flatten the model, erase the family boundary, or swap compile-time guarantees for runtime checks. It generates nicer constructor and pattern surfaces over the same nested-enum structure.

## Mental Model

- `#[nestum]` on an enum turns the enum name into a namespace for nested-path constructors.
- `nested! { ... }` rewrites nested constructors and nested patterns where Rust syntax needs help.
- `#[nestum_scope]` rewrites a whole function, impl, method, or inline module body so you do not need as many local `nested!` wrappers.
- If you need the concrete enum type itself, use `Outer::Enum<T>`.

That namespace tradeoff is what makes `Event::Document::Created` possible in ordinary Rust syntax.

## What This Path Means

- `DocumentEvent::Created` constructs a `DocumentEvent::Enum`.
- `Event::Document::Created` constructs an `Event::Enum`.
- `Event::Document` is a namespace branch, not a completed `Event` value.

Good fits for nestum look like:

- `Event::Document::Created`
- `Command::User::Create`
- `Message::Billing::Paid`

Weak fits usually look like:

- a one-off wrapper where helper functions already hide the wrapping noise
- a hierarchy you would have to invent just to get prettier syntax
- names like `Document::Event::Created` that read like inner type namespaces instead of outer envelope values

`nestum` is strongest when the outer enum is already a real envelope over event, command, or message families.

That usually means:

- the nested enums are already the most accurate model of the problem
- the family boundary is carrying real correctness information
- the pain is mostly constructor and match noise

## Quick Start

```bash
cargo add nestum
```

1. Add `#[nestum]` to each enum in the hierarchy.
2. Construct wrapped values with nested paths like `Event::Document::Created`.
3. Use `nested! { ... }` for focused rewrites, or put `#[nestum_scope]` on the enclosing function, impl, method, or inline module to rewrite a wider scope at once.

## Real-World Showcases

The [`nestum-examples`](./nestum-examples) workspace crate shows the macro against real libraries instead of toy enums.

- `todo_api`: Axum + in-memory SQLite + broadcast events. This keeps command families, domain errors, and emitted events as separate nested enum trees instead of flattening them for convenience.
- `ops_cli`: Clap subcommands with nested dispatch. This keeps the command tree honest at the type level while still letting the dispatch code read like the tree it models.

The example crate also shows the API-shape side of that approach: `todo_api` is split into `app`, `health`, and `todo` modules, and `ops_cli` exposes inner command families under `command::{User, Billing}` instead of repeating long top-level type prefixes.

Run them with:

```bash
cargo run -p nestum-examples --bin todo_api
cargo run -p nestum-examples --bin ops_cli -- users create dev@example.com
```

## Examples

### Basic Nesting

```rust
#[nestum]
enum DocumentEvent {
    Created,
    Deleted,
}

#[nestum]
enum ImageEvent {
    Uploaded,
    Archived,
}

#[nestum]
enum Event {
    Document(DocumentEvent),
    Image(ImageEvent),
}

let _ = Event::Document::Created;
let _ = Event::Image::Archived;
```

This is the core use case: keep the real family boundary in the type system, but stop paying for it with tuple-wrapping boilerplate.

### Named-Field Constructors

Use `nested!` when the nested leaf is a named-field variant:

```rust
use nestum::{nestum, nested};

#[nestum]
enum Inner {
    Struct { x: i32 },
}

#[nestum]
enum Outer {
    Wrap(Inner),
}

let value: Outer::Enum = nested! { Outer::Wrap::Struct { x: 5 } };
```

### Scope-Level Rewriting

Use `#[nestum_scope]` when a function or impl body has several nested constructors or patterns:

```rust
use nestum::{nestum, nestum_scope};

#[nestum]
enum DocumentEvent {
    Created,
    Renamed { title: &'static str },
}

#[nestum]
enum Event {
    Document(DocumentEvent),
}

#[nestum_scope]
fn handle(event: Event::Enum) {
    let renamed = Event::Document::Renamed { title: "scope" };
    assert!(matches!(
        renamed,
        Event::Document::Renamed { title } if title == "scope"
    ));

    match event {
        Event::Document::Created => {}
        Event::Document::Renamed { title } => {
            let _ = title;
        }
    }
}
```

### Cross-Module Nesting

Use `#[nestum(external = "...")]` when the inner enum lives in another module file:

```rust
mod inner;

#[nestum]
enum Outer {
    #[nestum(external = "crate::inner::Inner")]
    Wrap(Inner),
}

let _ = Outer::Wrap::A;
```

### Ecosystem Compatibility

`nestum` still works with common derive-heavy Rust crates. The test suite covers:

- `serde` round trips for wrapped outer enums
- `thiserror` derives with transparent outer error envelopes
- transitive `#[from]` conversions from leaf errors into nested outer error envelopes
- common assertion macros, including `assert!(matches!(...))`

## Core Rules

- Only enums are supported.
- Both the outer enum and the nested inner enum need `#[nestum]`.
- Use `nested!` for focused rewrites, or `#[nestum_scope]` for function-, impl-, method-, or inline-module-level rewrites.

## No Type-Safety Trade

`nestum` is syntax and namespace machinery over real nested enums.

- It keeps the same compile-time family boundaries.
- It does not weaken the invariant that an outer branch contains the right inner enum family.
- The tradeoffs are mostly in syntax, naming, and tooling, not in static correctness.

## Advanced Notes

- In type positions, use `Outer::Enum<T>` for the enum type itself.
- Put `#[nestum]` before `#[derive(...)]` so derive macros see the rewritten enum shape.
- `serde`, `thiserror`, and common assertion macros are covered by tests.
- When each nested error wrapper opts into `#[from]`, leaf errors can use `?` all the way into the outer envelope.
- Generic outer enums use functions for unit constructors, so `Outer::Other()` or `Outer::Wrap::Ready()` may be function calls instead of constants.
- Plain local inner enum paths, `self::...`, `super::...`, and qualified crate-local inner enum paths are supported, including generic arguments.
- Cross-module nesting is explicit with `#[nestum(external = "crate::path::Enum")]`.
- For nested variants, the raw root constructor path like `Outer::Wrap(inner)` is no longer part of the public surface; use `Outer::Enum::Wrap(inner)` if you need the explicit underlying constructor.
- `#[nestum_scope]` rewrites normal Rust AST inside the annotated item body and also handles `matches!`, `assert!`, `debug_assert!`, `assert_eq!`, `assert_ne!`, and their debug variants.

## Limitations

- Most other outer macro token trees are still opaque to `#[nestum_scope]`.
- qself or associated paths are rejected for nested field detection.
- `#[path = "..."]`, `include!()`, and complex `cfg` module layouts may not resolve.
- External crates are not supported because proc macros cannot reliably inspect dependency sources.

## API

### `#[nestum]`

Marks an enum so nested enum-wrapping variants can be constructed through path-shaped syntax.

```rust
use nestum::nestum;

#[nestum]
enum Inner {
    A,
    B,
}

#[nestum]
enum Outer {
    Wrap(Inner),
}

let _ = Outer::Wrap::A;
```

### `nested! { ... }`

Rewrites nested constructors and nested patterns into ordinary Rust enum syntax.
Use it for `match`, `if let`, `while let`, `let-else`, `matches!`, common assertion macros, and named-field nested construction.

```rust
use nestum::{nestum, nested};

#[nestum]
enum Inner {
    A,
    B(u8),
}

#[nestum]
enum Outer {
    Wrap(Inner),
}

let value = Outer::Wrap::B(3);
let ok = nested! { matches!(value, Outer::Wrap::B(n) if n > 0) };
```

### `#[nestum_scope]`

Rewrites nested constructors and nested patterns across a wider body.
Use it on functions, impl methods, impl blocks, or inline modules when local `nested!` wrappers would get noisy.

```rust
use nestum::{nestum, nestum_scope};

#[nestum]
enum Inner {
    A,
    B(u8),
}

#[nestum]
enum Outer {
    Wrap(Inner),
}

#[nestum_scope]
fn demo(value: Outer::Enum) -> bool {
    let created = Outer::Wrap::B(3);
    matches!(value, Outer::Wrap::A) && matches!(created, Outer::Wrap::B(n) if n > 0)
}
```

### `#[nestum(external = "path::to::Enum")]`

Marks a variant as wrapping a nested enum defined in another module file.

```rust
use nestum::nestum;

mod inner;

#[nestum]
enum Outer {
    #[nestum(external = "crate::inner::Inner")]
    Wrap(Inner),
}
```

### `nestum_match! { match value { ... } }`

Match-only compatibility macro. Prefer `nested!` unless you specifically want a `match`-only entry point.

## License

MIT
