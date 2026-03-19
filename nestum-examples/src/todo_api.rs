use axum::{
    Json, Router,
    extract::{Path, State},
    http,
    routing::{get, patch, post},
};
use nestum::nested;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

pub mod app;
pub mod health;
pub mod todo;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, FromRow)]
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
pub struct ErrorBody {
    pub error: &'static str,
    pub detail: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "data")]
pub enum ApiReply {
    Health(health::Response),
    Todo(Todo),
    Todos(Vec<Todo>),
}

#[nestum::nestum]
#[derive(Debug)]
pub enum ValidationError {
    EmptyTitle,
}

pub fn router(state: app::State) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/todos", get(list_todos).post(create_todo))
        .route("/todos/{id}/title", patch(rename_todo))
        .route("/todos/{id}/complete", post(complete_todo))
        .with_state(state)
}

async fn health(State(state): State<app::State>) -> Result<Json<ApiReply>, app::Error::Enum> {
    Ok(Json(app::Command::Health::Check.execute(&state).await?))
}

async fn list_todos(State(state): State<app::State>) -> Result<Json<ApiReply>, app::Error::Enum> {
    Ok(Json(app::Command::Todos::List.execute(&state).await?))
}

async fn create_todo(
    State(state): State<app::State>,
    Json(payload): Json<CreateTodoRequest>,
) -> Result<(http::StatusCode, Json<ApiReply>), app::Error::Enum> {
    let command = nested! {
        app::Command::Todos::Create {
            title: payload.title.try_into()?,
        }
    };
    Ok((
        http::StatusCode::CREATED,
        Json(command.execute(&state).await?),
    ))
}

async fn rename_todo(
    State(state): State<app::State>,
    Path(id): Path<i64>,
    Json(payload): Json<RenameTodoRequest>,
) -> Result<Json<ApiReply>, app::Error::Enum> {
    let command = nested! {
        app::Command::Todos::Rename {
            id,
            title: payload.title.try_into()?,
        }
    };
    Ok(Json(command.execute(&state).await?))
}

async fn complete_todo(
    State(state): State<app::State>,
    Path(id): Path<i64>,
) -> Result<Json<ApiReply>, app::Error::Enum> {
    Ok(Json(
        app::Command::Todos::Complete(id).execute(&state).await?,
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
        let state = app::State::in_memory().await.unwrap();
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
        let state = app::State::in_memory().await.unwrap();
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
        let state = app::State::in_memory().await.unwrap();
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
        let state = app::State::in_memory().await.unwrap();
        let mut events = state.subscribe();

        let reply = nested! {
            app::Command::Todos::Create {
                title: "Watch the event stream".to_string().try_into().unwrap(),
            }
        }
        .execute(&state)
        .await
        .unwrap();
        assert!(matches!(reply, ApiReply::Todo(_)));

        let event = events.recv().await.unwrap();
        nested! {
            match event {
                app::Event::Todos::Created(todo) => {
                    assert_eq!(todo.title, "Watch the event stream");
                }
                app::Event::Todos::Renamed(_) | app::Event::Todos::Completed(_) => {
                    panic!("unexpected event variant")
                }
            }
        }
    }

    #[test]
    fn event_summary_formats_nested_events() {
        let summary = app::Event::Todos::Completed(Todo {
            id: 9,
            title: "Ship docs".to_string(),
            completed: true,
        })
        .summary();

        assert_eq!(summary, "event: completed todo 9 (Ship docs)");
    }
}
