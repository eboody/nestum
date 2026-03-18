# nestum-examples

Real-world showcase crate for `nestum`.

- `todo_api`: Axum HTTP API backed by in-memory SQLite and a broadcast event stream.
- `ops_cli`: Clap-powered command tree with nested dispatch.

## What Each Showcase Proves

- `todo_api` shows that a normal service stack does not need to flatten its model to stay readable. Commands, validation errors, persistence errors, and emitted events remain separate nested enum families, so invalid cross-family states stay unrepresentable.
- `ops_cli` shows that a command tree can stay a real command tree. Clap parsing, type annotations, and dispatch all preserve the hierarchy instead of collapsing it into a flatter but less honest model.

## Why That Matters

`nestum` is not a new data model. These examples keep the same nested enums you would write for correctness:

- a todo command is not a validation error
- a billing command is not a user command
- an outer envelope branch can only contain the inner enum family it was defined to wrap

The crate only removes the wrapping noise that usually pushes people toward flatter, less precise representations.

## Run The API

Start the server:

```bash
cargo run -p nestum-examples --bin todo_api
```

Create a todo:

```bash
curl -s http://127.0.0.1:3000/todos \
  -H 'content-type: application/json' \
  -d '{"title":"Ship nestum examples"}'
```

Expected response:

```json
{"kind":"todo","data":{"id":1,"title":"Ship nestum examples","completed":false}}
```

List todos:

```bash
curl -s http://127.0.0.1:3000/todos
```

Expected response:

```json
{"kind":"todos","data":[{"id":1,"title":"Ship nestum examples","completed":false}]}
```

Complete a todo:

```bash
curl -s -X POST http://127.0.0.1:3000/todos/1/complete
```

Expected response:

```json
{"kind":"todo","data":{"id":1,"title":"Ship nestum examples","completed":true}}
```

The database is `sqlite::memory:`, so state resets when the process exits.

## Run The CLI

Create a user:

```bash
cargo run -p nestum-examples --bin ops_cli -- users create dev@example.com
```

Expected output:

```text
create-user:dev@example.com
```

Refund an invoice:

```bash
cargo run -p nestum-examples --bin ops_cli -- billing refund 42
```

Expected output:

```text
refund-invoice:42
```

## Test The Showcases

```bash
cargo test -p nestum-examples
```
