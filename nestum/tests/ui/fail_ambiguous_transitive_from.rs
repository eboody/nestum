use nestum::nestum;
use std::io;
use thiserror::Error;

#[nestum]
#[derive(Debug, Error)]
enum LeftError {
    #[error(transparent)]
    Io(#[from] io::Error),
}

#[nestum]
#[derive(Debug, Error)]
enum RightError {
    #[error(transparent)]
    Io(#[from] io::Error),
}

#[nestum]
#[derive(Debug, Error)]
enum AppError {
    #[error(transparent)]
    Left(#[from] LeftError),
    #[error(transparent)]
    Right(#[from] RightError),
}

fn main() {}
