use axum::{
    Json, Router,
    extract::{Path, State},
    http,
    response::IntoResponse,
    routing::{get, patch, post},
};
use nestum::{nested, nestum};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool, sqlite::SqlitePoolOptions};
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Todo {
    pub id: i64,
    pub title: String,
    pub completed: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateTodoRequest {
    pub title: String,
}

#[derive(Debug, Deserialize)]
pub struct RenameTodoRequest {
    pub title: String,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub todo_count: i64,
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: &'static str,
    pub detail: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "data")]
pub enum ApiReply {
    Health(HealthResponse),
    Todo(Todo),
    Todos(Vec<Todo>),
}

#[nestum]
#[derive(Debug, Clone)]
pub enum HealthCommand {
    Check,
}

#[nestum]
#[derive(Debug, Clone)]
pub enum TodoCommand {
    Create { title: String },
    Rename { id: i64, title: String },
    Complete(i64),
    List,
}

#[nestum]
#[derive(Debug, Clone)]
pub enum AppCommand {
    Health(HealthCommand),
    Todos(TodoCommand),
}

#[nestum]
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "topic", content = "payload", rename_all = "snake_case")]
pub enum TodoEvent {
    Created(Todo),
    Renamed(Todo),
    Completed(Todo),
}

#[nestum]
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "stream", content = "event", rename_all = "snake_case")]
pub enum AppEvent {
    Todos(TodoEvent),
}

#[nestum]
#[derive(Debug)]
pub enum ValidationError {
    EmptyTitle,
}

#[nestum]
#[derive(Debug)]
pub enum TodoError {
    NotFound(i64),
    Database(String),
}

#[nestum]
#[derive(Debug)]
pub enum AppError {
    Validation(ValidationError),
    Todos(TodoError),
}

#[derive(Clone)]
pub struct AppState {
    pool: SqlitePool,
    events: broadcast::Sender<AppEvent::Enum>,
}

impl AppState {
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

    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent::Enum> {
        self.events.subscribe()
    }

    fn publish(&self, event: AppEvent::Enum) {
        let _ = self.events.send(event);
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/todos", get(list_todos).post(create_todo))
        .route("/todos/{id}/title", patch(rename_todo))
        .route("/todos/{id}/complete", post(complete_todo))
        .with_state(state)
}

pub async fn execute(
    state: &AppState,
    command: AppCommand::Enum,
) -> Result<ApiReply, AppError::Enum> {
    nested! {
        match command {
            AppCommand::Health::Check => {
                let row = sqlx::query("SELECT COUNT(*) AS count FROM todos")
                    .fetch_one(&state.pool)
                    .await?;
                let todo_count: i64 = row.get("count");
                Ok(ApiReply::Health(HealthResponse {
                    status: "ok",
                    todo_count,
                }))
            }
            AppCommand::Todos::List => {
                let rows = sqlx::query("SELECT id, title, completed FROM todos ORDER BY id")
                    .fetch_all(&state.pool)
                    .await?;
                let todos = rows.into_iter().map(todo_from_row).collect();
                Ok(ApiReply::Todos(todos))
            }
            AppCommand::Todos::Create { title } => {
                let title = normalize_title(title)?;
                let result = sqlx::query("INSERT INTO todos (title, completed) VALUES (?, 0)")
                    .bind(&title)
                    .execute(&state.pool)
                    .await?;
                let todo = fetch_todo(&state.pool, result.last_insert_rowid()).await?;
                state.publish(AppEvent::Todos::Created(todo.clone()));
                Ok(ApiReply::Todo(todo))
            }
            AppCommand::Todos::Rename { id, title } => {
                let title = normalize_title(title)?;
                let result = sqlx::query("UPDATE todos SET title = ? WHERE id = ?")
                    .bind(&title)
                    .bind(id)
                    .execute(&state.pool)
                    .await?;
                if result.rows_affected() == 0 {
                    return Err(AppError::Todos::NotFound(id));
                }
                let todo = fetch_todo(&state.pool, id).await?;
                state.publish(AppEvent::Todos::Renamed(todo.clone()));
                Ok(ApiReply::Todo(todo))
            }
            AppCommand::Todos::Complete(id) => {
                let result = sqlx::query("UPDATE todos SET completed = 1 WHERE id = ?")
                    .bind(id)
                    .execute(&state.pool)
                    .await?;
                if result.rows_affected() == 0 {
                    return Err(AppError::Todos::NotFound(id));
                }
                let todo = fetch_todo(&state.pool, id).await?;
                state.publish(AppEvent::Todos::Completed(todo.clone()));
                Ok(ApiReply::Todo(todo))
            }
        }
    }
}

pub fn event_summary(event: AppEvent::Enum) -> String {
    nested! {
        match event {
            AppEvent::Todos::Created(todo) => {
                format!("event: created todo {} ({})", todo.id, todo.title)
            }
            AppEvent::Todos::Renamed(todo) => {
                format!("event: renamed todo {} ({})", todo.id, todo.title)
            }
            AppEvent::Todos::Completed(todo) => {
                format!("event: completed todo {} ({})", todo.id, todo.title)
            }
        }
    }
}

impl From<sqlx::Error> for AppError::Enum {
    fn from(error: sqlx::Error) -> Self {
        AppError::Todos::Database(error.to_string())
    }
}

impl IntoResponse for AppError::Enum {
    fn into_response(self) -> axum::response::Response {
        let (status, body) = nested! {
            match self {
                AppError::Validation::EmptyTitle => (
                    http::StatusCode::UNPROCESSABLE_ENTITY,
                    ErrorBody {
                        error: "validation",
                        detail: "title must not be blank".to_string(),
                    },
                ),
                AppError::Todos::NotFound(id) => (
                    http::StatusCode::NOT_FOUND,
                    ErrorBody {
                        error: "todo_not_found",
                        detail: format!("todo {id} does not exist"),
                    },
                ),
                AppError::Todos::Database(message) => (
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

fn normalize_title(title: String) -> Result<String, AppError::Enum> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err(AppError::Validation::EmptyTitle);
    }
    Ok(trimmed.to_string())
}

async fn fetch_todo(pool: &SqlitePool, id: i64) -> Result<Todo, AppError::Enum> {
    let row = sqlx::query("SELECT id, title, completed FROM todos WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;

    row.map(todo_from_row).ok_or(AppError::Todos::NotFound(id))
}

fn todo_from_row(row: sqlx::sqlite::SqliteRow) -> Todo {
    Todo {
        id: row.get("id"),
        title: row.get("title"),
        completed: row.get::<i64, _>("completed") != 0,
    }
}

async fn health(State(state): State<AppState>) -> Result<Json<ApiReply>, AppError::Enum> {
    Ok(Json(execute(&state, AppCommand::Health::Check).await?))
}

async fn list_todos(State(state): State<AppState>) -> Result<Json<ApiReply>, AppError::Enum> {
    Ok(Json(execute(&state, AppCommand::Todos::List).await?))
}

async fn create_todo(
    State(state): State<AppState>,
    Json(payload): Json<CreateTodoRequest>,
) -> Result<(http::StatusCode, Json<ApiReply>), AppError::Enum> {
    let command = nested! {
        AppCommand::Todos::Create {
            title: payload.title,
        }
    };
    Ok((
        http::StatusCode::CREATED,
        Json(execute(&state, command).await?),
    ))
}

async fn rename_todo(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(payload): Json<RenameTodoRequest>,
) -> Result<Json<ApiReply>, AppError::Enum> {
    let command = nested! {
        AppCommand::Todos::Rename {
            id,
            title: payload.title,
        }
    };
    Ok(Json(execute(&state, command).await?))
}

async fn complete_todo(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<ApiReply>, AppError::Enum> {
    Ok(Json(
        execute(&state, AppCommand::Todos::Complete(id)).await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http};
    use http_body_util::BodyExt;
    use serde_json::Value;
    use tower::ServiceExt;

    async fn json_body(response: axum::response::Response) -> Value {
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn todo_routes_work_against_in_memory_sqlite() {
        let state = AppState::in_memory().await.unwrap();
        let app = router(state);

        let create = app
            .clone()
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/todos")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"title":"Ship nestum examples"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create.status(), http::StatusCode::CREATED);

        let create_json = json_body(create).await;
        assert_eq!(create_json["kind"], "todo");
        assert_eq!(create_json["data"]["title"], "Ship nestum examples");
        assert_eq!(create_json["data"]["completed"], false);

        let list = app
            .clone()
            .oneshot(
                http::Request::builder()
                    .uri("/todos")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(list.status(), http::StatusCode::OK);

        let list_json = json_body(list).await;
        assert_eq!(list_json["kind"], "todos");
        assert_eq!(list_json["data"].as_array().unwrap().len(), 1);

        let complete = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/todos/1/complete")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(complete.status(), http::StatusCode::OK);

        let complete_json = json_body(complete).await;
        assert_eq!(complete_json["data"]["completed"], true);
    }

    #[tokio::test]
    async fn validation_and_not_found_errors_map_cleanly() {
        let state = AppState::in_memory().await.unwrap();
        let app = router(state);

        let blank_title = app
            .clone()
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/todos")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"title":"   "}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(blank_title.status(), http::StatusCode::UNPROCESSABLE_ENTITY);

        let missing = app
            .oneshot(
                http::Request::builder()
                    .method("PATCH")
                    .uri("/todos/42/title")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"title":"rename"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn health_route_reports_empty_database() {
        let state = AppState::in_memory().await.unwrap();
        let app = router(state);

        let health = app
            .oneshot(
                http::Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(health.status(), http::StatusCode::OK);

        let health_json = json_body(health).await;
        assert_eq!(health_json["kind"], "health");
        assert_eq!(health_json["data"]["status"], "ok");
        assert_eq!(health_json["data"]["todo_count"], 0);
    }

    #[tokio::test]
    async fn execute_publishes_nested_events() {
        let state = AppState::in_memory().await.unwrap();
        let mut events = state.subscribe();

        let reply = execute(
            &state,
            nested! {
                AppCommand::Todos::Create {
                    title: "Watch the event stream".to_string(),
                }
            },
        )
        .await
        .unwrap();
        assert!(matches!(reply, ApiReply::Todo(_)));

        let event = events.recv().await.unwrap();
        nested! {
            match event {
                AppEvent::Todos::Created(todo) => {
                    assert_eq!(todo.title, "Watch the event stream");
                }
                AppEvent::Todos::Renamed(_) | AppEvent::Todos::Completed(_) => {
                    panic!("unexpected event variant")
                }
            }
        }
    }

    #[test]
    fn event_summary_formats_nested_events() {
        let summary = event_summary(AppEvent::Todos::Completed(Todo {
            id: 9,
            title: "Ship docs".to_string(),
            completed: true,
        }));

        assert_eq!(summary, "event: completed todo 9 (Ship docs)");
    }
}
