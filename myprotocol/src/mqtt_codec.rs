use std::fmt;

use anyhow::{anyhow, Result};
use bytes::{BufMut, BytesMut};
use mqttbytes::{v5::*, PacketType};
use rand::{distributions::Alphanumeric, Rng, SeedableRng};
use rand_chacha::ChaCha20Rng;
use tracing::{info, warn};

pub struct MqttCodec {
    pub max_packet_size: usize,
}

// move to shared/mqtt/codec.rs
impl MqttCodec {
    pub fn encode(&self, packet: &Packet) -> Result<Vec<u8>> {
        let mut buffer: BytesMut = BytesMut::new();
        let n = match packet {
            Packet::Connect(connect) => connect.write(&mut buffer),
            Packet::ConnAck(conn_ack) => conn_ack.write(&mut buffer),
            Packet::Publish(publish) => publish.write(&mut buffer),
            Packet::PubAck(pub_ack) => pub_ack.write(&mut buffer),
            Packet::PubRec(pub_rec) => pub_rec.write(&mut buffer),
            Packet::PubRel(pub_rel) => pub_rel.write(&mut buffer),
            Packet::PubComp(pub_comp) => pub_comp.write(&mut buffer),
            Packet::Subscribe(subscribe) => subscribe.write(&mut buffer),
            Packet::SubAck(sub_ack) => sub_ack.write(&mut buffer),
            Packet::Unsubscribe(unsubscribe) => unsubscribe.write(&mut buffer),
            Packet::UnsubAck(unsub_ack) => unsub_ack.write(&mut buffer),
            Packet::PingReq => PingReq.write(&mut buffer),
            Packet::PingResp => PingResp.write(&mut buffer),
            Packet::Disconnect(disconnect) => disconnect.write(&mut buffer),
        }
        .map_err(|e| anyhow::anyhow!(e))?;

        Ok(buffer.to_vec())
    }

    pub fn decode(&self, buffer: &mut BytesMut) -> Result<Option<Packet>> {
        match read(buffer, self.max_packet_size) {
            Ok(packet) => Ok(Some(packet)),
            Err(mqttbytes::Error::InsufficientBytes(_)) => Ok(None), // Partial data, wait for more
            Err(e) => Err(anyhow::anyhow!(e)),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub struct ClientID(String);

impl Default for ClientID {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientID {
    pub fn new() -> Self {
        let mut rng = ChaCha20Rng::from_entropy();
        let random_id = rng
            .sample_iter(&Alphanumeric)
            .take(23)
            .map(char::from)
            .collect();
        Self(random_id)
    }

    pub fn get(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ClientID {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<ClientID> for String {
    fn from(client_id: ClientID) -> Self {
        client_id.0
    }
}

/// According to MQTT-5.0-3.1.3.1
impl TryFrom<String> for ClientID {
    type Error = anyhow::Error;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() || value.len() > 23 {
            Err(anyhow!(
                "ClientID has invalid length: {}. Must be between 1 - 23 charaters.",
                value.len()
            ))
        } else if !value.chars().all(|c| c.is_ascii_alphanumeric()) {
            Err(anyhow!(
                "ClientID contains invalid characters. Only alphanumeric characters are allowed."
            ))
        } else {
            Ok(Self(value.to_string()))
        }
    }
}