# nestum

**nestum** makes nested enum paths and matches ergonomic in Rust, so you can encode invariants with nested enums without paying a readability tax.

```rust
use nestum::{nestum, nested};

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

Instead of:
```rust
Event::Documents(DocumentsEvent::Update(doc))
```

you can write:
```rust
Event::Documents::Update(doc)
```

## Why Use nestum?
- **Codify invariants**: express “this variant always contains this family of sub-variants.”
- **Readable matches**: nested paths keep intent obvious in large match statements.
- **Low boilerplate**: minimal annotations, no custom enums or manual conversions.

Where this pays off:
- Event routing and message buses.
- Permission or policy trees (resource + action).
- Parsers and ASTs (node + kind).
- UIs with nested state machines.

## Table of Contents
- [Quick Start](#quick-start)
- [Features & Examples](#features--examples)
  - [Basic Nesting](#1-basic-nesting)
  - [Nested Match Patterns](#2-nested-match-patterns)
  - [Cross-Module Nesting](#3-cross-module-nesting)
- [Common Errors and Tips](#common-errors-and-tips)
- [API Reference](#api-reference)
- [License](#license)

## Quick Start

```rust
use nestum::nestum;

#[derive(Debug)]
pub struct Document {
    pub id: String,
}

#[nestum]
pub enum DocumentsEvent {
    Update(Document),
    Delete(String),
}

#[nestum]
pub enum Event {
    Documents(DocumentsEvent),
}

fn main() {
    let doc = Document { id: "doc-1".to_string() };
    let _ = Event::Documents::Update(doc);
}
```

## Features & Examples

### 1. Basic Nesting
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

### 2. Nested Match Patterns
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
        Event::Documents::Update(doc) => { let _ = doc.id; }
        Event::Documents::Delete(id) => { let _ = id; }
        Event::Images::Update(img) => { let _ = img.id; }
        Event::Images::Delete(id) => { let _ = id; }
    }
}
```

### 3. Cross-Module Nesting
```rust
mod inner;

#[nestum]
pub enum Outer {
    #[nestum(external = "crate::inner::Inner")]
    Wrap(Inner),
}

let _ = Outer::Wrap::A;
```

## Common Errors and Tips
- **Only enums are supported**: `#[nestum]` must be on an enum.
- **External enums require an explicit path**: use `#[nestum(external = "crate::path::Enum")]`.
- **Nested enums must be marked**: both the parent and inner enum must have `#[nestum]`.
- **Unsupported layouts**: `#[path = "..."]`, `include!()`, and complex `cfg` module layouts may not resolve.
- **External crates** are not supported (proc macros cannot reliably inspect dependency sources).

## API Reference

### `#[nestum]` on enums
Enables nested paths and match rewriting.

### `#[nestum(external = "path::to::Enum")]` on variants
Opt-in support for nesting an enum in another module file.

### `nestum_match! { match value { ... } }` / `nested! { match value { ... } }`
Rewrites nested patterns (like `Event::Documents::Update`) into real enum patterns.

## License
MIT
