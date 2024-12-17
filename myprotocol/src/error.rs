use quinn::{ClosedStream, WriteError};

#[derive(Debug)]
pub enum ServerError {
    MqttError(mqttbytes::Error),
    QuinnWriteError(WriteError),
    QuinnClosedStreamError(ClosedStream),
    // other variants
}

impl std::error::Error for ServerError {}

impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}
