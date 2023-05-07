use thiserror::Error;
use miette::Diagnostic;

use crate::message::Message;

#[derive(Debug, Error, Diagnostic)]
pub enum BmailError{
    #[error("Internal Server Error")]
    InternalServerError,
    #[error(transparent)]
    ConfigError(#[from] config::ConfigError),
    #[error(transparent)]
    SerdeCborError(#[from] serde_cbor::Error),
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    #[error(transparent)]
    DecryptError(#[from] age::DecryptError),
    #[error(transparent)]
    FromStringError(#[from] std::string::FromUtf8Error),
    #[error("Tokio Send Error {0}")]
    TokioSendError(String),
}

impl From< tokio::sync::mpsc::error::SendError<Message>> for BmailError{
    fn from(value: tokio::sync::mpsc::error::SendError<Message>) -> Self {
        Self::TokioSendError(value.to_string())
    }
}