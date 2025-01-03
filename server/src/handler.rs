use crate::state::ServerState;
use anyhow::Result;
use mqttbytes::{v5::*, QoS};
use myprotocol::ClientID;
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{info, warn};

pub struct BrokerConfig {
    pub max_qos: QoS,
}

pub struct PacketHandler;

impl PacketHandler {
    pub async fn process_first_packet(
        packet: Packet,
        config: &BrokerConfig,
        state: &ServerState,
    ) -> Result<Option<(ClientID, Sender<Packet>, Receiver<Packet>)>> {
        match packet {
            Packet::Connect(connect) => Self::process_connect(connect, config.max_qos, state).await,
            _ => Ok(None),
        }
    }

    pub async fn process_packet(
        packet: Packet,
        config: &BrokerConfig,
        state: &ServerState,
        client_id: &ClientID,
        sender: &Sender<Packet>,
    ) -> Result<bool> {
        info!("Processing: {:?}", packet);
        match packet {
            Packet::Connect(_) => Self::handle_invalid_packet(sender).await,
            Packet::ConnAck(_) => Self::handle_invalid_packet(sender).await,
            Packet::Publish(publish) => {
                Self::process_publish(publish, state, config.max_qos, sender).await
            }
            Packet::PubAck(_) => Self::handle_invalid_packet(sender).await,
            Packet::PubRec(_) => Self::handle_invalid_packet(sender).await,
            Packet::PubRel(_) => Self::handle_invalid_packet(sender).await,
            Packet::PubComp(_) => Self::handle_invalid_packet(sender).await,
            Packet::Subscribe(subscribe) => {
                Self::process_subscribe(subscribe, state, config.max_qos, client_id, sender).await
            }
            Packet::SubAck(_) => Self::handle_invalid_packet(sender).await,
            Packet::Unsubscribe(unsubscribe) => {
                Self::process_unsubscribe(unsubscribe, state, client_id, sender).await
            }
            Packet::UnsubAck(_) => Self::handle_invalid_packet(sender).await,
            Packet::PingReq => Self::process_ping_req(sender).await,
            Packet::PingResp => Self::handle_invalid_packet(sender).await,
            Packet::Disconnect(disconnect) => {
                Self::process_disconnect(disconnect, state, client_id).await
            }
        }
    }

    async fn process_connect(
        connect: Connect,
        max_qos: QoS,
        state: &ServerState,
    ) -> Result<Option<(ClientID, Sender<Packet>, Receiver<Packet>)>> {
        let client_id = ClientID::try_from(connect.client_id)?;
        if let Some(will) = &connect.last_will {
            if will.qos > max_qos {
                // Will QoS exceeds max allowed QoS; reject the connection
                info!("Client requested last will QoS that exceeds max allowed QoS; rejecting the connection");
                return Ok(None);
            }
        }

        let (sender, receiver) = state.add_client(&client_id).await;
        sender
            .send(Packet::ConnAck(ConnAck::new(
                ConnectReturnCode::Success,
                false,
            )))
            .await?;

        Ok(Some((client_id, sender, receiver)))
    }

    async fn handle_invalid_packet(sender: &Sender<Packet>) -> Result<bool> {
        sender
            .send(Packet::Disconnect(Disconnect {
                reason_code: DisconnectReasonCode::ProtocolError,
                properties: None,
            }))
            .await?;
        Ok(true)
    }

    async fn process_disconnect(
        _: Disconnect,
        state: &ServerState,
        client_id: &ClientID,
    ) -> Result<bool> {
        state.remove_client(client_id).await;
        Ok(true)
    }

    async fn process_ping_req(sender: &Sender<Packet>) -> Result<bool> {
        sender.send(Packet::PingResp).await?;
        Ok(false)
    }

    async fn process_publish(
        publish: Publish,
        state: &ServerState,
        max_qos: QoS,
        sender: &Sender<Packet>,
    ) -> Result<bool> {
        if publish.qos > max_qos {
            sender
                .send(Packet::Disconnect(Disconnect {
                    reason_code: DisconnectReasonCode::QoSNotSupported,
                    properties: None,
                }))
                .await?;
            return Ok(true);
        }

        let clients = state.clients.read().await;
        let senders = clients.iter().filter_map(|(client_id, client_info)| {
            client_info
                .subscriptions
                .contains(&publish.topic)
                .then_some((client_id.clone(), client_info.sender.clone()))
        });
        for (client_id, sender) in senders {
            if let Err(e) = sender.send(Packet::Publish(publish.clone())).await {
                warn!("Failed to publish to {}: {}", client_id, e);
            }
        }

        Ok(false)
    }

    async fn process_subscribe(
        subscribe: Subscribe,
        state: &ServerState,
        max_qos: QoS,
        client_id: &ClientID,
        sender: &Sender<Packet>,
    ) -> Result<bool> {
        let mut return_codes = Vec::new();

        for filter in subscribe.filters {
            let topic = filter.path.clone();
            let granted_qos = match filter.qos {
                qos if qos > max_qos => match max_qos {
                    QoS::AtMostOnce => SubscribeReasonCode::QoS0,
                    QoS::AtLeastOnce => SubscribeReasonCode::QoS1,
                    QoS::ExactlyOnce => SubscribeReasonCode::QoS2,
                },
                qos => match qos {
                    QoS::AtMostOnce => SubscribeReasonCode::QoS0,
                    QoS::AtLeastOnce => SubscribeReasonCode::QoS1,
                    QoS::ExactlyOnce => SubscribeReasonCode::QoS2,
                },
            };
            return_codes.push(granted_qos);
            state.add_subscription(topic, client_id).await;
        }

        sender
            .send(Packet::SubAck(SubAck::new(subscribe.pkid, return_codes)))
            .await?;

        Ok(false)
    }

    async fn process_unsubscribe(
        unsubscribe: Unsubscribe,
        state: &ServerState,
        client_id: &ClientID,
        sender: &Sender<Packet>,
    ) -> Result<bool> {
        let mut reasons = Vec::new();

        for filter in unsubscribe.filters {
            let removed = state.remove_subscription(filter, client_id).await;
            reasons.push(match removed {
                true => UnsubAckReason::Success,
                false => UnsubAckReason::NoSubscriptionExisted,
            });
        }

        sender
            .send(Packet::UnsubAck(UnsubAck {
                pkid: unsubscribe.pkid,
                reasons,
                properties: None,
            }))
            .await?;

        Ok(false)
    }
}
