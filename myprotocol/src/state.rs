use std::collections::HashMap;

use mqttbytes::v5::{ConnAck, ConnAckProperties, ConnectReturnCode, PubAck, Publish};
use tokio::sync::{mpsc, RwLock};
use tracing::info;

use crate::{ClientID, ServerError};

// Represents per-connection outgoing messages
pub enum OutgoingMessage {
    ConnAck(ConnAck),
    Publish(Publish),
    PubAck(PubAck), // You can add more message types if needed
}

pub struct ClientState {
    pub client_id: ClientID,
    // We use a Sender<OutgoingMessage> to push outgoing messages to this client
    sender: mpsc::Sender<OutgoingMessage>,
    pub subscribed_topics: Vec<(String, u8)>, // topic + QoS
                                              // subscribed_topics, QoS info, etc.
}

pub struct ServerState {
    clients: RwLock<HashMap<ClientID, ClientState>>,
}

impl Default for ServerState {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerState {
    pub fn new() -> Self {
        Self {
            clients: RwLock::new(HashMap::new()),
        }
    }

    pub async fn subscribers_for(&self, topic: &str) -> Vec<mpsc::Sender<OutgoingMessage>> {
        let map = self.clients.read().await;
        let mut result = Vec::new();
        for client in map.values() {
            if client.subscribed_topics.iter().any(|(t, _)| t == topic) {
                result.push(client.sender.clone());
            }
        }
        result
    }

    pub async fn second_connect_error(&self, client_id: &ClientID) -> Result<(), ServerError> {
        let mut map = self.clients.write().await;
        if let Some(client) = map.get_mut(client_id) {
            client
                .sender
                .send(OutgoingMessage::ConnAck(ConnAck {
                    code: ConnectReturnCode::ProtocolError,
                    session_present: false,
                    properties: None,
                }))
                .await
                .map_err(ServerError::SendError)?;

            Ok(())
        } else {
            Err(ServerError::NoSuchClientError(client_id.clone()))
        }
    }

    pub async fn add_client(
        &self,
        client_id: &ClientID,
        sender: mpsc::Sender<OutgoingMessage>,
    ) -> Result<(), ServerError> {
        // Send connack
        sender
            .send(OutgoingMessage::ConnAck(ConnAck {
                code: ConnectReturnCode::Success,
                session_present: false,
                properties: Some(ConnAckProperties::new()),
            }))
            .await
            .map_err(ServerError::SendError)?;

        let mut map = self.clients.write().await;
        map.insert(
            client_id.clone(),
            ClientState {
                client_id: client_id.clone(),
                sender,
                subscribed_topics: vec![],
            },
        );

        Ok(())
    }

    pub async fn remove_client(&self, client_id: &ClientID) {
        let mut map = self.clients.write().await;
        map.remove(client_id);
    }

    pub async fn send_published_to_subscribed(&self, topic: &str, payload: Vec<u8>) {}

    pub async fn add_subscription(
        &self,
        client_id: &ClientID,
        topic: &str,
        qos: u8,
    ) -> Result<(), ServerError> {
        let mut map = self.clients.write().await;
        if let Some(client) = map.get_mut(client_id) {
            // Check if already subscribed or just push new
            // For simplicity:
            client.subscribed_topics.push((topic.to_string(), qos));

            client
                .sender
                .send(OutgoingMessage::PubAck(PubAck::new(1)))
                .await;

            Ok(())
        } else {
            Err(ServerError::StringError(format!("No such client: {:?}", client_id)))
        }
    }

    pub async fn send_publish(&self, client_id: &ClientID, publish: Publish) -> Result<(), String> {
        let map = self.clients.read().await;
        if let Some(client) = map.get(client_id) {
            client
                .sender
                .send(OutgoingMessage::Publish(publish))
                .await
                .map_err(|_| "Client disconnected".to_string())
        } else {
            Err("No such client".to_string())
        }
    }

    pub async fn handle_publish(
        &self,
        _publish_packet: &mqttbytes::v5::Publish,
    ) -> Result<(), crate::error::ServerError> {
        // Extract the topic from the publish packet
        let topic = &_publish_packet.topic;

        // Retrieve all subscribers of this topic
        let subscribers = self.subscribers_for(topic).await;

        // For each subscriber, send the message via its channel
        for subscriber in subscribers {
            // Construct an OutgoingMessage::Publish variant
            let msg = OutgoingMessage::Publish(_publish_packet.clone());
            // Sent it. If the subscriber disconnected, this may fail
            if let Err(_e) = subscriber.send(msg).await {
                // The subscriber might have disconnected
                // You could log or handle this, or ignore silently
            }
        }

        Ok(())
    }
}
