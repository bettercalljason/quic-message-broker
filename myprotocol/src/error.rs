use quinn::{ClosedStream, ConnectionError, WriteError};
use tokio::sync::mpsc::{self, error::SendError};

use crate::{ClientID, OutgoingMessage};

#[derive(Debug)]
pub enum ServerError {
    MqttError(mqttbytes::Error),
    SendError(SendError<OutgoingMessage>),
    NoSuchClientError(ClientID),
    QuinnWriteError(WriteError),
    QuinnClosedStreamError(ClosedStream),
    QuinnConnectionError(ConnectionError),
    StringError(String), // other variants
}

impl std::error::Error for ServerError {}

impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}
