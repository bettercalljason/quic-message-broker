use std::collections::HashMap;

use mqttbytes::v5::Packet;
use shared::mqtt::ClientID;
use tokio::sync::{
    mpsc::{self, Receiver, Sender},
    RwLock,
};
use tracing::info;

use crate::auth::{AuthStore, User};

pub struct ClientInfo {
    pub subscriptions: Vec<String>,
    pub sender: mpsc::Sender<Packet>,
}

pub struct ServerState {
    pub clients: RwLock<HashMap<ClientID, ClientInfo>>,
    pub auth_store: RwLock<AuthStore>,
}

impl ServerState {
    pub fn new() -> Self {
        let mut users = HashMap::new();
        users.insert(
            "jason".to_string(),
            User::new(
                "jason".to_string(),
                "supersecure".to_string(),
                vec!["mytopic".to_string()],
                vec!["mysub".to_string()],
            ),
        );

        Self {
            clients: RwLock::new(HashMap::new()),
            auth_store: RwLock::new(AuthStore::new(users)),
        }
    }

    pub async fn add_client(&self, client_id: &ClientID) -> (Sender<Packet>, Receiver<Packet>) {
        info!("Adding client {client_id}");
        let mut map = self.clients.write().await;
        let (sender, receiver) = mpsc::channel(100);

        map.insert(
            client_id.clone(),
            ClientInfo {
                subscriptions: Vec::new(),
                sender: sender.clone(),
            },
        );

        (sender, receiver)
    }

    pub async fn remove_client(&self, client_id: &ClientID) {
        info!("Removing client {client_id}");
        let mut map = self.clients.write().await;
        map.remove(client_id);
    }

    pub async fn add_subscription(&self, topic: String, client_id: &ClientID) {
        info!("Adding subscription {topic} for client {client_id}");
        let mut map = self.clients.write().await;
        let client = map.get_mut(client_id).expect("No such client");
        client.subscriptions.push(topic);
    }

    pub async fn remove_subscription(&self, topic: String, client_id: &ClientID) -> bool {
        info!("Removing subscription {topic} for client {client_id}");
        let mut map = self.clients.write().await;
        let client = map.get_mut(client_id).expect("No such client");

        if client.subscriptions.contains(&topic) {
            client.subscriptions.retain_mut(|x| *x != topic);
            true
        } else {
            false
        }
    }
}
