# nestum

`nestum` makes nested enum paths and matches feel natural, so you can codify invariants by nesting related enums while keeping ergonomic construction and matching.

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

Where this pays off:
- Event routing and message buses.
- Permission or policy trees (resource + action).
- Parsers/compilers (node + kind).
- UIs with nested state machines.

The point is to encode invariants as nested enums (this variant always contains this family of sub-variants) without paying a readability or ergonomics tax when constructing or matching.

## How To Use

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

## Cross-Module Nesting
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

## API Summary

### `#[nestum]` on enums
Enables nested paths and match rewriting.

### `#[nestum(external = "path::to::Enum")]` on variants
Opt-in support for nesting an enum in another module file.

### `nestum_match! { match value { ... } }` / `nested! { match value { ... } }`
Macro that rewrites nested patterns (like `Event::Documents::Update`) into real enum patterns.

## Limitations
- External crates are not supported (proc macros can’t reliably inspect other crates’ ASTs).
- The macro only resolves enums from source files in the current crate.
- `#[path = "..."]`, `include!()`, and complex `cfg` layouts may not be resolved.

## License
MIT
