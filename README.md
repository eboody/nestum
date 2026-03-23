<div align="center">
  <img alt="nestum logo" src="https://raw.githubusercontent.com/eboody/nestum/main/nestum.png" width="360">
  <p>Nestum keeps the honest nested-enum model and removes the wrapping noise.</p>
  <p>
    <a href="https://github.com/eboody/nestum/actions/workflows/ci.yml"><img src="https://github.com/eboody/nestum/actions/workflows/ci.yml/badge.svg?branch=main&event=push" alt="build status" /></a>
    <a href="https://crates.io/crates/nestum"><img src="https://img.shields.io/crates/v/nestum.svg?logo=rust" alt="crates.io" /></a>
    <a href="https://docs.rs/nestum"><img src="https://docs.rs/nestum/badge.svg" alt="docs.rs" /></a>
  </p>
</div>

# Nestum

If your Rust code already has real command, event, or error trees, `nestum` is for the annoying part: constructing and matching them.

The model is usually right already. The syntax is what gets old:

Before:

```rust
state.publish(Event::Todos(todo::Event::Created(todo.clone())));
return Err(Error::Todos(todo::Error::NotFound(id)));

match self {
    Error::Validation(ValidationError::EmptyTitle) => { /* ... */ }
    Error::Todos(todo::Error::NotFound(id)) => { /* ... */ }
    Error::Todos(todo::Error::Database(message)) => { /* ... */ }
}
```

After:

```rust
state.publish(Event::Todos::Created(todo.clone()));
return Err(Error::Todos::NotFound(id));

match self {
    Error::Validation::EmptyTitle => { /* ... */ }
    Error::Todos::NotFound(id) => { /* ... */ }
    Error::Todos::Database(message) => { /* ... */ }
}
```

That is the whole pitch. `nestum` keeps the same nested-enum model, keeps the same compile-time invariant, and removes most of the tuple-wrapping tax.

## When Nestum Is Worth It

Use `nestum` when all of these are true:

- the outer enum is already a real envelope over command, event, message, or error families
- that family boundary carries real correctness information
- you construct and match those envelopes often enough that wrapper syntax is now the main pain
- you want to keep normal derive-heavy Rust enums instead of flattening the model

Strong fits usually look like:

- error envelopes
- command trees
- event and message trees

## Do Not Use Nestum If...

- you would invent a hierarchy just to get prettier syntax
- the outer enum is a one-off wrapper and helper functions already hide the noise
- flattening the model would actually be clearer for the domain
- the nesting path depends on `#[cfg]`, `#[cfg_attr]`, `include!()`, `#[path = "..."]`, or macro-generated local enums
- the nested inner enum lives in an external crate

## Flagship Use Case

The strongest example in this repo is [`nestum-examples/src/todo_api/app.rs`](./nestum-examples/src/todo_api/app.rs).

It keeps three separate nested trees at the application boundary:

- `Command` for the work the API can perform
- `Event` for the domain events it emits
- `Error` for validation and persistence failures

That boundary looks like this:

```rust
#[nestum]
#[derive(Debug, Clone)]
pub enum Command {
    Health(super::health::Command),
    Todos(super::todo::Command),
}

#[nestum]
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "stream", content = "event", rename_all = "snake_case")]
pub enum Event {
    Todos(super::todo::Event),
}

#[nestum]
#[derive(Debug)]
pub enum Error {
    Validation(super::ValidationError),
    Todos(super::todo::Error),
}
```

That shape stays honest all the way through the app:

- route handlers build nested commands
- the app service matches nested command families
- response mapping matches nested error families
- event publishing emits nested event families

`nestum` is most valuable when the same tree shows up across several of those call sites, not just one constructor.

## Quick Start

```bash
cargo add nestum
```

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

let event: Event::Enum = Event::Document::Created;

nested! {
    match event {
        Event::Document::Created => {}
        Event::Document::Deleted => {}
    }
}
```

## Migration Guide

The `todo_api` example is a good migration model because it uses nested enums at a real boundary instead of in a toy demo.

Start with the honest nested enums you already have:

```rust
#[nestum]
pub enum Error {
    Validation(super::ValidationError),
    Todos(super::todo::Error),
}
```

Then change the call sites that currently pay the wrapper tax.

Before:

```rust
let command = app::Command::Todos(todo::Command::Create {
    title: payload.title.try_into()?,
});

state.publish(Event::Todos(todo::Event::Created(todo.clone())));

match self {
    Error::Validation(ValidationError::EmptyTitle) => (
        http::StatusCode::UNPROCESSABLE_ENTITY,
        ErrorBody {
            error: "validation",
            detail: "title must not be blank".to_string(),
        },
    ),
    Error::Todos(todo::Error::NotFound(id)) => (
        http::StatusCode::NOT_FOUND,
        ErrorBody {
            error: "todo_not_found",
            detail: format!("todo {id} does not exist"),
        },
    ),
    Error::Todos(todo::Error::Database(message)) => (
        http::StatusCode::INTERNAL_SERVER_ERROR,
        ErrorBody {
            error: "database",
            detail: message,
        },
    ),
}
```

After:

```rust
let command = nested! {
    app::Command::Todos::Create {
        title: payload.title.try_into()?,
    }
};

state.publish(Event::Todos::Created(todo.clone()));

nested! {
    match self {
        Error::Validation::EmptyTitle => (
            http::StatusCode::UNPROCESSABLE_ENTITY,
            ErrorBody {
                error: "validation",
                detail: "title must not be blank".to_string(),
            },
        ),
        Error::Todos::NotFound(id) => (
            http::StatusCode::NOT_FOUND,
            ErrorBody {
                error: "todo_not_found",
                detail: format!("todo {id} does not exist"),
            },
        ),
        Error::Todos::Database(message) => (
            http::StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody {
                error: "database",
                detail: message,
            },
        ),
    }
}
```

The data model did not change. The envelope shape did not change. The syntax got closer to the tree you were already modeling.

## Cookbooks

### `thiserror`: Nested Error Envelopes

`nestum` works well when an outer error envelope preserves the error family boundary and `thiserror` handles display, source chaining, and `#[from]`.

```rust
use nestum::{nestum, nested};
use thiserror::Error;

#[nestum]
#[derive(Debug, Error)]
pub enum DocumentError {
    #[error("document not found")]
    NotFound,
    #[error("invalid title: {0}")]
    InvalidTitle(String),
}

#[nestum]
#[derive(Debug, Error)]
pub enum ApiError {
    #[error(transparent)]
    Document(#[from] DocumentError),
    #[error("transport error")]
    Transport,
}

let err: ApiError::Enum = DocumentError::InvalidTitle("draft".to_string()).into();

let ok = nested! {
    matches!(err, ApiError::Document::InvalidTitle(title) if title == "draft")
};
assert!(ok);
```

The test suite also covers transitive `#[from]` through nested error trees and rejects ambiguous conversions.

### `clap`: Command Trees

The [`ops_cli`](./nestum-examples/src/ops_cli.rs) example keeps the command hierarchy honest and lets dispatch read like the CLI tree.

```rust
#[nestum]
#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    #[command(subcommand)]
    Users(command::User),
    #[command(subcommand)]
    Billing(command::Billing),
}

nested! {
    match self {
        Command::Users::Create(args) => format!("create-user:{}", args.email),
        Command::Users::Suspend { user_id } => format!("suspend-user:{user_id}"),
        Command::Billing::Charge(args) => {
            format!("charge-invoice:{}:{}c", args.invoice_id, args.cents)
        }
        Command::Billing::Refund { invoice_id } => format!("refund-invoice:{invoice_id}"),
    }
}
```

This is the kind of command surface where `nestum` tends to pay for itself quickly.

### `serde`: Preserve the Envelope Shape

`nestum` does not flatten nested enums before serialization. `serde` still sees the real wrapped structure.

```rust
#[nestum]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocumentEvent {
    Created { id: u64 },
    Renamed { title: String },
}

#[nestum]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Event {
    Document(DocumentEvent),
    Health,
}
```

A nested value like `Event::Document::Renamed { title: "Spec".to_string() }` serializes as:

```json
{
  "Document": {
    "Renamed": {
      "title": "Spec"
    }
  }
}
```

That makes `nestum` a good fit when the nested structure itself matters on the wire.

### Axum: Response Mapping and Route Commands

The `todo_api` example shows both sides:

- route handlers build nested commands from request payloads
- `IntoResponse` matches nested error branches directly

```rust
let command = nested! {
    app::Command::Todos::Rename {
        id,
        title: payload.title.try_into()?,
    }
};

let (status, body) = nested! {
    match self {
        Error::Validation::EmptyTitle => (
            http::StatusCode::UNPROCESSABLE_ENTITY,
            ErrorBody {
                error: "validation",
                detail: "title must not be blank".to_string(),
            },
        ),
        Error::Todos::NotFound(id) => (
            http::StatusCode::NOT_FOUND,
            ErrorBody {
                error: "todo_not_found",
                detail: format!("todo {id} does not exist"),
            },
        ),
        Error::Todos::Database(message) => (
            http::StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody {
                error: "database",
                detail: message,
            },
        ),
    }
};
```

This is a strong pattern when handlers, services, and error mapping all share the same command or error tree.

## Mental Model

- `#[nestum]` turns an enum name into a namespace for nested-path constructors.
- `nested! { ... }` rewrites nested constructors and nested patterns where Rust syntax needs help.
- `#[nestum_scope]` rewrites a whole function, impl, method, or inline module body when local `nested!` wrappers would get noisy.
- `Outer::Enum<T>` is the underlying enum type when you need it in a type position.

`Event::Document::Created` is not a flattened replacement for the underlying enum. It is syntax over the same nested model.

## Real-World Examples

The [`nestum-examples`](./nestum-examples) workspace crate includes:

- `todo_api`: Axum + SQLite-backed todo API with nested commands, events, and errors
- `ops_cli`: Clap command tree with nested dispatch

Run them with:

```bash
cargo run -p nestum-examples --bin todo_api
cargo run -p nestum-examples --bin ops_cli -- users create dev@example.com
```

## No Type-Safety Trade

`nestum` is syntax and namespace machinery over real nested enums.

- it keeps the same compile-time family boundaries
- it does not replace those boundaries with strings or runtime tags
- it keeps derive-heavy enums compatible with the rest of the ecosystem

## Authority Surface

Within its supported observation point, `nestum` treats parsed crate-local source plus proc-macro source locations as authoritative for nested-path expansion.

That means:

- source locations for proc-macro expansion must be available
- every module and enum on the nesting path must be directly present in parsed crate-local source
- `#[cfg]` and `#[cfg_attr]` on modules, enums, variants, or enum fields are rejected for nesting resolution
- `#[path = "..."]`, `include!()`, and macro-generated local enums are outside that authority surface

Unsupported cases are rejected where `nestum` can detect them. When source-location context is unavailable, `nestum` now errors instead of guessing.

## API

### `#[nestum]`

Marks an enum so nested enum-wrapping variants can be constructed through path-shaped syntax.

### `nested! { ... }`

Rewrites nested constructors and nested patterns into ordinary Rust enum syntax.

Use it for:

- `match`
- `if let`
- `while let`
- `let-else`
- `matches!`
- `assert!`, `debug_assert!`, `assert_eq!`, `assert_ne!`, and debug variants
- named-field nested construction

### `#[nestum_scope]`

Rewrites nested constructors and nested patterns across a wider body.

Use it on:

- functions
- impl methods
- impl blocks
- inline modules

### `#[nestum(external = "path::to::Enum")]`

Marks a variant as wrapping a nested enum defined in another crate-local module file.

### `nestum_match! { match value { ... } }`

Match-only compatibility macro.

Prefer `nested!` unless you specifically want a `match`-only entry point.

## Limitations

- `nestum` inspects parsed crate-local source plus proc-macro source locations, not macro-expanded or type-checked items
- external crates are not supported as nested inner enums because proc macros cannot reliably inspect dependency sources
- `macro_rules!`-generated local enums are not supported as nested inner enums
- `#[cfg]`, `#[cfg_attr]`, `#[path = "..."]`, and `include!()` are unsupported on the nesting path
- most other outer macro token trees are still opaque to `#[nestum_scope]`
- qself or associated paths are rejected for nested field detection

## License

MIT
