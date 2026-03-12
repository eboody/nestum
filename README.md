# nestum

`nestum` lets nested enums read like nested paths in Rust.

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

## Mental Model

- `#[nestum]` on an enum turns the enum name into a namespace for nested-path constructors.
- `nested! { ... }` rewrites nested constructors and nested patterns where Rust syntax needs help.
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

Avoid shapes like `Document::Event::Created` as the main mental model. They read like inner type namespaces, but nestum is strongest when the outer enum is an envelope over event, command, or message families.

## Quick Start

```bash
cargo add nestum
```

1. Add `#[nestum]` to each enum in the hierarchy.
2. Construct wrapped values with nested paths like `Event::Document::Created`.
3. Wrap `match`, `if let`, `while let`, `let-else`, `matches!`, and named-field nested construction in `nested! { ... }`.

## Real-World Showcases

The [`nestum-examples`](./nestum-examples) workspace crate shows the macro against real libraries instead of toy enums.

- `todo_api`: Axum + in-memory SQLite + broadcast events. This proves nested command trees, nested domain errors, and nested events can survive a normal web stack.
- `ops_cli`: Clap subcommands with nested dispatch. This proves nestum can sit on top of derive-heavy command surfaces without turning the type tree into boilerplate.

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

## Core Rules

- Only enums are supported.
- Both the outer enum and the nested inner enum need `#[nestum]`.
- Use `nested!` for pattern-bearing forms and named-field nested constructors.

## Advanced Notes

- In type positions, use `Outer::Enum<T>` for the enum type itself.
- Put `#[nestum]` before `#[derive(...)]` so derive macros see the rewritten enum shape.
- Generic outer enums use functions for nested unit constructors, so `Outer::Wrap::Ready()` may be a function call instead of a constant.
- Plain or qualified local inner enum paths are supported, including generic arguments.
- Cross-module nesting is explicit with `#[nestum(external = "crate::path::Enum")]`.
- For nested variants, the raw root constructor path like `Outer::Wrap(inner)` is no longer part of the public surface; use `Outer::Enum::Wrap(inner)` if you need the explicit underlying constructor.

## Limitations

- `self::...`, `super::...`, and qself or associated paths are rejected for nested field detection.
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
Use it for `match`, `if let`, `while let`, `let-else`, `matches!`, and named-field nested construction.

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
