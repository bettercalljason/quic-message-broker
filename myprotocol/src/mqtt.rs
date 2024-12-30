use std::{
    fmt::{self, Display},
    sync::Arc,
};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use bytes::BytesMut;
use mqttbytes::{v5::*, PacketType};
use quinn::SendStream;
use rand::{distributions::Alphanumeric, rngs::OsRng, thread_rng, Rng, SeedableRng};
use rand_chacha::ChaCha20Rng;
use tokio::sync::mpsc;
use tracing::{error, info};

use crate::{protocol::ProtocolHandler, state::ServerState, OutgoingMessage};

#[derive(std::fmt::Debug)]
pub enum MqttEvent {
    ClientConnected { client_id: ClientID },
    PublishReceived { topic: String, payload: Vec<u8> },
    ClientSubscribed { topic: String, qos: u8 },
    ClientDisconnected,
    // ... other events
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
        write!(f, "ClientID {}", self.0)
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

pub struct MqttHandler {
    max_packet_size: usize,
}

impl MqttHandler {
    pub fn new(max_packet_size: usize) -> Self {
        MqttHandler { max_packet_size }
    }

    fn handle_packet(&self, packet: Packet, server_state: &Arc<ServerState>) -> Result<MqttEvent> {
        match packet {
            Packet::Connect(p) => Ok(MqttEvent::ClientConnected {
                client_id: ClientID::try_from(p.client_id)?,
            }),
            Packet::Disconnect(p) => Ok(MqttEvent::ClientDisconnected),
            Packet::Subscribe(p) => Ok(MqttEvent::ClientSubscribed {
                topic: "foo".to_string(),
                qos: 1,
            }),
            _ => todo!(),
        }
    }

    /// Reads a stream of bytes and extracts next MQTT packet out of it
    /// Patched version of mqttbytes::v5::read, because that one did not accept Disconnects without payload
    pub fn read_patched(
        &self,
        stream: &mut BytesMut,
        max_size: usize,
    ) -> Result<Packet, mqttbytes::Error> {
        let fixed_header = mqttbytes::check(stream.iter(), max_size)?;

        // Test with a stream with exactly the size to check border panics
        let packet = stream.split_to(fixed_header.frame_length());
        let packet_type = fixed_header.packet_type()?;

        // if fixed_header.remaining_len == 0 {
        //     // no payload packets
        //     return match packet_type {
        //         PacketType::PingReq => Ok(Packet::PingReq),
        //         PacketType::PingResp => Ok(Packet::PingResp),
        //         _ => Err(Error::PayloadRequired),
        //     };
        // }

        let packet = packet.freeze();
        let packet = match packet_type {
            PacketType::Connect => Packet::Connect(Connect::read(fixed_header, packet)?),
            PacketType::ConnAck => Packet::ConnAck(ConnAck::read(fixed_header, packet)?),
            PacketType::Publish => Packet::Publish(Publish::read(fixed_header, packet)?),
            PacketType::PubAck => Packet::PubAck(PubAck::read(fixed_header, packet)?),
            PacketType::PubRec => Packet::PubRec(PubRec::read(fixed_header, packet)?),
            PacketType::PubRel => Packet::PubRel(PubRel::read(fixed_header, packet)?),
            PacketType::PubComp => Packet::PubComp(PubComp::read(fixed_header, packet)?),
            PacketType::Subscribe => Packet::Subscribe(Subscribe::read(fixed_header, packet)?),
            PacketType::SubAck => Packet::SubAck(SubAck::read(fixed_header, packet)?),
            PacketType::Unsubscribe => {
                Packet::Unsubscribe(Unsubscribe::read(fixed_header, packet)?)
            }
            PacketType::UnsubAck => Packet::UnsubAck(UnsubAck::read(fixed_header, packet)?),
            PacketType::PingReq => Packet::PingReq,
            PacketType::PingResp => Packet::PingResp,
            PacketType::Disconnect => Packet::Disconnect(Disconnect::read(fixed_header, packet)?),
        };

        Ok(packet)
    }

    fn try_parse_packet(&self, buf: &mut BytesMut) -> Result<Option<Packet>> {
        match self.read_patched(buf, self.max_packet_size) {
            Ok(packet) => Ok(Some(packet)),
            Err(mqttbytes::Error::InsufficientBytes(_)) => Ok(None),
            Err(e) => Err(anyhow!("Reading error: {:?}", e)),
        }
    }
}

#[async_trait]
impl ProtocolHandler for MqttHandler {
    async fn handle_bytes(
        &self,
        buf: &mut BytesMut,
        server_state: &Arc<ServerState>,
    ) -> Result<Vec<MqttEvent>> {
        let mut events = Vec::new();

        while let Some(packet) = self.try_parse_packet(buf)? {
            let event = self.handle_packet(packet, server_state)?;
            info!("Event {:?}", event);
            events.push(event);
        }

        Ok(events)
    }
}
