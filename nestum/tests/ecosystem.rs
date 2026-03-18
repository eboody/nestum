use nestum::{nested, nestum};
use serde::{Deserialize, Serialize};
use std::{error::Error as _, io};
use thiserror::Error;

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

#[nestum]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
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

#[nestum]
#[derive(Debug, Error)]
pub enum LeafError {
    #[error(transparent)]
    Io(#[from] io::Error),
}

#[nestum]
#[derive(Debug, Error)]
pub enum ServiceError {
    #[error(transparent)]
    Leaf(#[from] LeafError),
    #[error("storage failed at {path}")]
    Storage {
        path: &'static str,
        #[source]
        source: io::Error,
    },
}

#[nestum]
#[derive(Debug, Error)]
pub enum OuterError {
    #[error(transparent)]
    Service(#[from] ServiceError),
}

#[test]
fn serde_round_trip_preserves_nested_envelope_shape() {
    let event: Event::Enum = nested! {
        Event::Document::Renamed {
            title: "Spec".to_string(),
        }
    };

    let json = serde_json::to_value(&event).expect("event should serialize");
    assert_eq!(
        json,
        serde_json::json!({
            "Document": {
                "Renamed": {
                    "title": "Spec"
                }
            }
        })
    );

    let decoded: Event::Enum = serde_json::from_value(json).expect("event should deserialize");
    assert_eq!(decoded, event);

    let ok = nested! {
        matches!(decoded, Event::Document::Renamed { title } if title == "Spec")
    };
    assert!(ok);
}

#[test]
fn thiserror_transparent_and_from_work_with_nested_enums() {
    let err: ApiError::Enum = DocumentError::InvalidTitle("draft".to_string()).into();

    assert_eq!(err.to_string(), "invalid title: draft");

    let ok = nested! {
        matches!(err, ApiError::Document::InvalidTitle(title) if title == "draft")
    };
    assert!(ok);
}

#[test]
fn transitive_from_reaches_the_outer_error_envelope() {
    fn fail() -> Result<(), OuterError::Enum> {
        Err(io::Error::other("disk broke"))?;
        Ok(())
    }

    let err = fail().expect_err("io leaf error should convert into the outer envelope");
    assert_eq!(err.to_string(), "disk broke");

    let ok = nested! {
        matches!(err, OuterError::Service::Leaf::Io(source) if source.to_string() == "disk broke")
    };
    assert!(ok);
}

#[test]
fn named_field_nested_errors_keep_their_source_chain() {
    let err: OuterError::Enum = nested! {
        OuterError::Service::Storage {
            path: "todos.db",
            source: io::Error::other("disk broke"),
        }
    };

    assert_eq!(err.to_string(), "storage failed at todos.db");
    let source = err
        .source()
        .expect("named-field source should be preserved");
    assert_eq!(source.to_string(), "disk broke");
}
