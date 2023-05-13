use miette::Diagnostic;
use thiserror::Error;

use crate::message::FirehoseMessages;

#[derive(Debug, Error, Diagnostic)]
pub enum BmailError {
    #[error("Internal Server Error")]
    InternalServerError,
    #[error("Firehose Process Crashed")]
    FirehoseProcessCrashed,
    #[error("Conversation Not Found")]
    ConversationNotFound,
    #[error("Malformed Bmail")]
    MalformedBmail,
    #[error("Missing Session")]
    MissingSession,
    #[error("Missing Identity")]
    MissingIdentity,
    #[error("Missing Recipient Identity")]
    MissingRecipientIdentity,
    #[error("Missing Recipient {0}")]
    MissingRecipient(String),
    #[error("Missing Recipient Keys")]
    MultipleRecipientKeys,
    #[error(transparent)]
    ConfigError(#[from] config::ConfigError),
    #[error(transparent)]
    SerdeCborError(#[from] serde_cbor::Error),
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    #[error(transparent)]
    DecryptError(#[from] age::DecryptError),
    #[error(transparent)]
    EncryptError(#[from] age::EncryptError),
    #[error(transparent)]
    FromStringError(#[from] std::string::FromUtf8Error),
    #[error(transparent)]
    Base64DecodeError(#[from] base64::DecodeError),
    #[error(transparent)]
    CborDecodeError(#[from] ciborium::de::Error<std::io::Error>),
    #[error(transparent)]
    CborEncodeError(#[from] ciborium::ser::Error<std::io::Error>),
    #[error("Tokio Send Error {0}")]
    TokioSendError(String),
    #[error("Stream Error")]
    StreamError,
    #[error(transparent)]
    BiskyError(#[from] bisky::errors::BiskyError),
    #[error("Failed to Parse Recipient Key String")]
    ParseRecipientError,
}

impl From<tokio::sync::mpsc::error::SendError<FirehoseMessages>> for BmailError {
    fn from(value: tokio::sync::mpsc::error::SendError<FirehoseMessages>) -> Self {
        Self::TokioSendError(value.to_string())
    }
}

impl From<bisky::atproto::StreamError> for BmailError{
    fn from(_value: bisky::atproto::StreamError) -> Self {
        Self::StreamError
    }
}
