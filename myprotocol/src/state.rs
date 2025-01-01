use std::collections::HashMap;

use anyhow::{anyhow, Result};
use mqttbytes::{
    v5::{ConnAck, ConnAckProperties, ConnectReturnCode, Disconnect, PubAck, Publish},
    QoS,
};
use tokio::sync::{mpsc, RwLock};
use tracing::info;

use crate::ClientID;

// Represents per-connection outgoing messages
pub enum OutgoingMessage {
    ConnAck(ConnAck),
    Disconnect(Disconnect),
    Publish(Publish),
}

pub struct ClientState {
    pub client_id: ClientID,
    // We use a Sender<OutgoingMessage> to push outgoing messages to this client
    sender: mpsc::Sender<OutgoingMessage>,
    pub subscribed_topics: Vec<String>,
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
            if client.subscribed_topics.iter().any(|t| t == topic) {
                result.push(client.sender.clone());
            }
        }
        result
    }

    pub async fn second_connect_error(&self, client_id: &ClientID) -> Result<()> {
        let mut map = self.clients.write().await;
        if let Some(client) = map.get_mut(client_id) {
            let mut properties = ConnAckProperties::new();
            properties.max_qos = Some(0); // At most once
            let properties = properties;

            client
                .sender
                .send(OutgoingMessage::ConnAck(ConnAck {
                    code: ConnectReturnCode::ProtocolError,
                    session_present: false,
                    properties: Some(properties),
                }))
                .await?;

            Ok(())
        } else {
            Err(anyhow!("No such client: {:?}", client_id))
        }
    }

    pub async fn send_connack_with_qos_not_supported(&self, client_id: &ClientID) -> Result<()> {
        let mut map = self.clients.write().await;
        if let Some(client) = map.get_mut(client_id) {
            let mut properties = ConnAckProperties::new();
            properties.max_qos = Some(0); // At most once
            let properties = properties;

            client
                .sender
                .send(OutgoingMessage::ConnAck(ConnAck {
                    code: ConnectReturnCode::QoSNotSupported,
                    session_present: false,
                    properties: Some(properties),
                }))
                .await?;

            Ok(())
        } else {
            Err(anyhow!("No such client: {:?}", client_id))
        }
    }

    pub async fn send_disconnect_with_qos_not_supported(&self, client_id: &ClientID) -> Result<()> {
        let mut map = self.clients.write().await;
        if let Some(client) = map.get_mut(client_id) {
            let mut properties = ConnAckProperties::new();
            properties.max_qos = Some(0); // At most once
            let properties = properties;

            client
                .sender
                .send(OutgoingMessage::Disconnect(Disconnect {
                    properties: None,
                    reason_code: mqttbytes::v5::DisconnectReasonCode::QoSNotSupported,
                }))
                .await?;

            Ok(())
        } else {
            Err(anyhow!("No such client: {:?}", client_id))
        }
    }

    pub async fn add_client(
        &self,
        client_id: &ClientID,
        sender: mpsc::Sender<OutgoingMessage>,
    ) -> Result<()> {
        let mut properties = ConnAckProperties::new();
        properties.max_qos = Some(0); // At most once
        let properties = properties;

        sender
            .send(OutgoingMessage::ConnAck(ConnAck {
                code: ConnectReturnCode::Success,
                session_present: false,
                properties: Some(properties),
            }))
            .await?;

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

    pub async fn add_subscription(&self, client_id: &ClientID, topic: &str) -> Result<()> {
        let mut map = self.clients.write().await;
        if let Some(client) = map.get_mut(client_id) {
            // TODO: Check if already subscribed or just push new
            client.subscribed_topics.push(topic.to_string());

            Ok(())
        } else {
            Err(anyhow!("No such client: {:?}", client_id))
        }
    }

    pub async fn remove_subscription(&self, client_id: &ClientID, topic: &str) -> Result<()> {
        let mut map = self.clients.write().await;
        if let Some(client) = map.get_mut(client_id) {
            client.subscribed_topics.retain(|t| t != topic);

            Ok(())
        } else {
            Err(anyhow!("No such client: {:?}", client_id))
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

    pub async fn handle_publish(&self, _publish_packet: &mqttbytes::v5::Publish) -> Result<()> {
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
