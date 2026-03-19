use axum::{Json, http, response::IntoResponse};
use nestum::{nested, nestum};
use serde::Serialize;
use sqlx::{Row, SqlitePool, sqlite::SqlitePoolOptions};
use tokio::sync::broadcast;

use super::{ApiReply, ErrorBody, Todo, ValidationError, health, todo::Title};

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

#[derive(Clone)]
pub struct State {
    pool: SqlitePool,
    events: broadcast::Sender<Event::Enum>,
}

impl State {
    pub async fn in_memory() -> Result<Self, sqlx::Error> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        sqlx::query(
            "CREATE TABLE todos (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                completed INTEGER NOT NULL DEFAULT 0
            )",
        )
        .execute(&pool)
        .await?;
        let (events, _) = broadcast::channel(32);
        Ok(Self { pool, events })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event::Enum> {
        self.events.subscribe()
    }

    fn publish(&self, event: Event::Enum) {
        let _ = self.events.send(event);
    }

    async fn todo(&self, id: i64) -> Result<Todo, Error::Enum> {
        let row = sqlx::query_as::<_, Todo>("SELECT id, title, completed FROM todos WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        row.ok_or(Error::Todos::NotFound(id))
    }
}

impl Event::Enum {
    pub fn summary(&self) -> String {
        nested! {
            match self {
                Event::Todos::Created(todo) => {
                    format!("event: created todo {} ({})", todo.id, todo.title)
                }
                Event::Todos::Renamed(todo) => {
                    format!("event: renamed todo {} ({})", todo.id, todo.title)
                }
                Event::Todos::Completed(todo) => {
                    format!("event: completed todo {} ({})", todo.id, todo.title)
                }
            }
        }
    }
}

impl From<ValidationError::Enum> for Error::Enum {
    fn from(error: ValidationError::Enum) -> Self {
        Error::Enum::Validation(error)
    }
}

impl From<sqlx::Error> for Error::Enum {
    fn from(error: sqlx::Error) -> Self {
        Error::Todos::Database(error.to_string())
    }
}

impl IntoResponse for Error::Enum {
    fn into_response(self) -> axum::response::Response {
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

        (status, Json(body)).into_response()
    }
}

impl Command::Enum {
    pub async fn execute(self, state: &State) -> Result<ApiReply, Error::Enum> {
        nested! {
            match self {
                Command::Health::Check => {
                    let row = sqlx::query("SELECT COUNT(*) AS count FROM todos")
                        .fetch_one(&state.pool)
                        .await?;
                    let todo_count: i64 = row.try_get("count")?;
                    Ok(ApiReply::Health(health::Response {
                        status: "ok",
                        todo_count,
                    }))
                }
                Command::Todos::List => {
                    let todos = sqlx::query_as::<_, Todo>(
                        "SELECT id, title, completed FROM todos ORDER BY id",
                    )
                    .fetch_all(&state.pool)
                    .await?;
                    Ok(ApiReply::Todos(todos))
                }
                Command::Todos::Create { title } => {
                    let result = sqlx::query("INSERT INTO todos (title, completed) VALUES (?, 0)")
                        .bind(title.as_ref())
                        .execute(&state.pool)
                        .await?;
                    let todo = state.todo(result.last_insert_rowid()).await?;
                    state.publish(Event::Todos::Created(todo.clone()));
                    Ok(ApiReply::Todo(todo))
                }
                Command::Todos::Rename { id, title } => {
                    let result = sqlx::query("UPDATE todos SET title = ? WHERE id = ?")
                        .bind(title.as_ref())
                        .bind(id)
                        .execute(&state.pool)
                        .await?;
                    if result.rows_affected() == 0 {
                        return Err(Error::Todos::NotFound(id));
                    }
                    let todo = state.todo(id).await?;
                    state.publish(Event::Todos::Renamed(todo.clone()));
                    Ok(ApiReply::Todo(todo))
                }
                Command::Todos::Complete(id) => {
                    let result = sqlx::query("UPDATE todos SET completed = 1 WHERE id = ?")
                        .bind(id)
                        .execute(&state.pool)
                        .await?;
                    if result.rows_affected() == 0 {
                        return Err(Error::Todos::NotFound(id));
                    }
                    let todo = state.todo(id).await?;
                    state.publish(Event::Todos::Completed(todo.clone()));
                    Ok(ApiReply::Todo(todo))
                }
            }
        }
    }
}
