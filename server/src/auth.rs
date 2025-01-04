use std::collections::HashMap;
use subtle::ConstantTimeEq;
use anyhow::{anyhow, Result};

pub struct User {
    username: String,
    password: String, // TODO: Store hashed value in a database
    allowed_filters: Vec<String>,
    allowed_topics: Vec<String>,
}

impl User {
    pub fn new(
        username: String,
        password: String,
        allowed_filters: Vec<String>,
        allowed_topics: Vec<String>,
    ) -> Self {
        Self {
            username,
            password,
            allowed_filters,
            allowed_topics,
        }
    }

    pub fn can_publish(&self, topic: String) -> bool {
        self.allowed_topics.contains(&topic)
    }

    pub fn can_subscribe(&self, filter: String) -> bool {
        self.allowed_filters.contains(&filter)
    }
}

pub struct AuthStore {
    pub users: HashMap<String, User>,
}

impl AuthStore {
    pub fn new() -> Self {
        Self {
            users: HashMap::new(),
        }
    }

    pub fn can_user_publish(&self, username: &str, topic: &str) -> Result<bool> {
        self.users.get(username).ok_or(anyhow!("User does not exist")).map(|user| user.can_publish(topic.to_string()))
    }

    pub fn can_user_subscribe(&self, username: &str, filter: &str) -> Result<bool> {
        self.users.get(username).ok_or(anyhow!("User does not exist")).map(|user| user.can_subscribe(filter.to_string()))
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
