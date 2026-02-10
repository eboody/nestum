# nestum

`nestum` is a proc-macro that makes *nested enum paths and matches* feel natural, so you can write:

```rust
Event::Documents::Update
```

instead of:

```rust
Event::Documents(DocumentsEvent::Update(...))
```

It does this by generating a shadow module hierarchy and wrapper constructors around your enums, plus a match macro that rewrites nested patterns.

## Why
Rust enums are great for modeling state and variants, but nesting them gets noisy fast:

```rust
Event::Documents(DocumentsEvent::Update(doc))
```

`nestum` removes the clutter by letting you access nested variants via paths:

```rust
Event::Documents::Update(doc)
```

The payoff is biggest in **matching**, where you can read intent at a glance:

```rust
nested! {
    match event {
        Event::Documents::Update(doc) => { /* ... */ }
        Event::Images::Delete(id) => { /* ... */ }
    }
}
```

You still get the same enum types and semantics—just much cleaner call sites.

Other places this shines:
- Event routing (webhooks, message buses, job systems).
- Permission or policy trees (resource + action).
- Parsers or compilers (node + node-kind).
- UIs with nested state machines.

## Quick Start

```rust
use nestum::nestum;

#[derive(Debug)]
pub struct Document {
    pub id: String,
}

#[derive(Debug)]
pub struct Image {
    pub id: String,
}

#[nestum]
pub enum DocumentsEvent {
    Update(Document),
    Delete(String),
}

#[nestum]
pub enum ImagesEvent {
    Update(Image),
    Delete(String),
}

#[nestum]
pub enum Event {
    Documents(DocumentsEvent),
    Images(ImagesEvent),
}

fn main() {
    let doc = Document { id: "doc-1".to_string() };
    let _ = Event::Documents::Update(doc);
    let _ = Event::Images::Delete("img-1".to_string());
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

### `nestum_match! { match value { ... } }` / `nested! { match value { ... } }`
Macro that rewrites nested patterns (like `Enum1::Variant1::VariantA`) into real enum patterns.

## Examples

### Basic nesting
```rust
#[nestum]
pub enum DocumentsEvent { Update(Document), Delete(String) }

#[nestum]
pub enum ImagesEvent { Update(Image), Delete(String) }

#[nestum]
pub enum Event { Documents(DocumentsEvent), Images(ImagesEvent) }

let _ = Event::Documents::Update(doc);
let _ = Event::Images::Delete("img-1".to_string());
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

### Nested match patterns (front and center)
```rust
use nestum::{nestum, nested};

#[nestum]
pub enum DocumentsEvent { Update(Document), Delete(String) }

#[nestum]
pub enum ImagesEvent { Update(Image), Delete(String) }

#[nestum]
pub enum Event { Documents(DocumentsEvent), Images(ImagesEvent) }

let event = Event::Documents::Update(Document { id: "doc-1".to_string() });
nested! {
    match event {
        Event::Documents::Update(doc) => {
            let _ = doc.id;
        }
        Event::Documents::Delete(id) => {
            let _ = id;
        }
        Event::Images::Update(img) => {
            let _ = img.id;
        }
        Event::Images::Delete(id) => {
            let _ = id;
        }
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
Use `nested!` (or `nestum_match!`) so paths like `Event::Documents::Update` are rewritten to the real enum pattern.

### Why not support external crates?
The macro would need to discover and parse dependency source files, which is brittle and not reliably possible in stable proc-macro APIs.

## License
MIT
