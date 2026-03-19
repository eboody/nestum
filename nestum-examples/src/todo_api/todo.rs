use nestum::nestum;
use serde::Serialize;

use super::{Todo, ValidationError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Title(String);

#[nestum]
#[derive(Debug, Clone)]
pub enum Command {
    Create { title: Title },
    Rename { id: i64, title: Title },
    Complete(i64),
    List,
}

#[nestum]
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "topic", content = "payload", rename_all = "snake_case")]
pub enum Event {
    Created(Todo),
    Renamed(Todo),
    Completed(Todo),
}

#[nestum]
#[derive(Debug)]
pub enum Error {
    NotFound(i64),
    Database(String),
}

impl TryFrom<String> for Title {
    type Error = ValidationError::Enum;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(ValidationError::EmptyTitle);
        }

        Ok(Self(trimmed.to_string()))
    }
}

impl AsRef<str> for Title {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
