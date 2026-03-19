use nestum::nestum;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Response {
    pub status: &'static str,
    pub todo_count: i64,
}

#[nestum]
#[derive(Debug, Clone)]
pub enum Command {
    Check,
}
