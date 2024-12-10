use std::collections::HashMap;

use tokio::sync::RwLock;

pub struct ClientState {
    pub client_id: String,
    // subscribed_topics, QoS info, etc.
}

pub struct ServerState {
    clients: RwLock<HashMap<String, ClientState>>,
}

impl ServerState {
    pub fn new() -> Self {
        Self {
            clients: RwLock::new(HashMap::new()),
        }
    }

    pub async fn add_client(&self, client_id: &str) {
        let mut map = self.clients.write().await;
        map.insert(
            client_id.to_string(),
            ClientState {
                client_id: client_id.to_string(),
            },
        );
    }

    pub async fn handle_publish(
        &self,
        _publish_packet: &mqttbytes::v5::Publish,
    ) -> Result<(), crate::error::ServerError> {
        // Distribute message to subscribed clients
        Ok(())
    }
}
