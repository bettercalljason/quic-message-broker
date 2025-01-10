use anyhow::{anyhow, Result};
use mqttbytes::matches;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use subtle::ConstantTimeEq;

#[derive(Deserialize, Debug)]
pub struct User {
    password: String,
    allowed_filters: HashSet<String>,
    allowed_topics: HashSet<String>,
}

impl User {
    pub fn can_publish(&self, topic: String) -> bool {
        self.allowed_topics.iter().any(|t| matches(&topic, t))
    }

    pub fn can_subscribe(&self, filter: String) -> bool {
        self.allowed_filters.iter().any(|f| matches(&filter, f))
    }
}

pub struct AuthStore {
    pub users: HashMap<String, User>,
}

impl AuthStore {
    pub fn new(users: HashMap<String, User>) -> Self {
        Self { users }
    }

    pub fn can_user_publish(&self, username: &str, topic: &str) -> Result<bool> {
        self.users
            .get(username)
            .ok_or(anyhow!("User does not exist"))
            .map(|user| user.can_publish(topic.to_string()))
    }

    pub fn can_user_subscribe(&self, username: &str, filter: &str) -> Result<bool> {
        self.users
            .get(username)
            .ok_or(anyhow!("User does not exist"))
            .map(|user| user.can_subscribe(filter.to_string()))
    }

    pub fn is_login_valid(&self, username: &str, password: String) -> bool {
        if let Some(user) = self.users.get(username) {
            if bool::from(user.password.as_bytes().ct_eq(password.as_bytes())) {
                return true;
            }
        }
        false
    }
}
