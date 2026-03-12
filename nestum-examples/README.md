# nestum-examples

Real-world showcase crate for `nestum`.

- `todo_api`: Axum HTTP API backed by in-memory SQLite and a broadcast event stream.
- `ops_cli`: Clap-powered command tree with nested dispatch.

## What Each Showcase Proves

- `todo_api` shows nestum in a normal service stack: request routing, domain commands, domain errors, database work, and emitted events all stay in nested enums.
- `ops_cli` shows nestum in a derive-heavy CLI surface: nested subcommands still dispatch like the conceptual tree they model.

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
