use std::collections::HashMap;

use myprotocol::ClientID;
use tracing::info;

pub struct ClientInfo {
    pub subscriptions: Vec<String>,
}

pub struct ServerState {
    pub clients: HashMap<ClientID, ClientInfo>,
}

impl ServerState {
    pub fn new() -> Self {
        Self {
            clients: HashMap::new(),
        }
    }

    pub fn add_client(&mut self, client_id: &ClientID) {
        info!("Adding client {client_id}");
        self.clients.insert(
            client_id.clone(),
            ClientInfo {
                subscriptions: Vec::new(),
            },
        );
    }

    pub fn remove_client(&mut self, client_id: &ClientID) {
        info!("Removing client {client_id}");
        self.clients.remove(client_id);
    }

    pub fn add_subscription(&mut self, topic: String, client_id: &ClientID) {
        info!("Adding subscription {topic} for client {client_id}");
        let client = self.clients.get_mut(client_id).expect("No such client");
        client.subscriptions.push(topic);
    }

    pub fn remove_subscription(&mut self, topic: String, client_id: &ClientID) -> bool {
        info!("Removing subscription {topic} for client {client_id}");
        let client = self.clients.get_mut(client_id).expect("No such client");

        if client.subscriptions.contains(&topic) {
            client.subscriptions.retain_mut(|x| *x != topic);
            true
        } else {
            false
        }
    }
}
