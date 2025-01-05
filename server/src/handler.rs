use crate::state::ServerState;
use anyhow::Result;
use mqttbytes::{matches, v5::*, valid_filter, valid_topic, PacketType, QoS};
use shared::mqtt::{packet_type, ClientID};
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{info, warn};

pub struct BrokerConfig {
    pub max_qos: QoS,
    pub keep_alive: u16, // Set it to the Transport's max idle timeout: https://github.com/quinn-rs/quinn/blob/f5b1ec7dd96c9b56ef98f2a7a91acaf5e341d718/quinn-proto/src/config/transport.rs#L331
}

pub struct PacketHandler;

impl PacketHandler {
    pub async fn process_first_packet(
        packet: Packet,
        config: &BrokerConfig,
        state: &ServerState,
    ) -> Result<(ClientID, String, Sender<Packet>, Receiver<Packet>)> {
        match packet {
            Packet::Connect(connect) => {
                let client_id = ClientID::try_from(connect.client_id)?;
                if let Some(will) = &connect.last_will {
                    if will.qos > config.max_qos {
                        return Err(anyhow::anyhow!(
                            "Client requested last will QoS that exceeds max allowed QoS"
                        ));
                    }
                }

                let username = match connect.login {
                    Some(login) => {
                        let auth_store = state.auth_store.read().await;
                        if !auth_store.is_login_valid(&login.username, login.password) {
                            return Err(anyhow::anyhow!("Invalid username or password"));
                        } else {
                            login.username.clone()
                        }
                    }
                    None => {
                        return Err(anyhow::anyhow!("Missing login credentials"));
                    }
                };

                let (sender, receiver) = state.add_client(&client_id).await;

                let mut properties = ConnAckProperties::new();
                properties.max_qos = Some(match config.max_qos {
                    QoS::AtMostOnce => 0,
                    QoS::AtLeastOnce => 1,
                    QoS::ExactlyOnce => 2,
                });
                properties.server_keep_alive = Some(config.keep_alive);
                properties.shared_subscription_available = Some(0);
                properties.topic_alias_max = Some(0);
                properties.retain_available = Some(0);

                match sender
                    .send(Packet::ConnAck(ConnAck {
                        code: ConnectReturnCode::Success,
                        session_present: false,
                        properties: Some(properties),
                    }))
                    .await
                {
                    Ok(_) => {}
                    Err(e) => {
                        state.remove_client(&client_id).await;
                        return Err(anyhow::anyhow!(e));
                    }
                }

                Ok((client_id, username, sender, receiver))
            }
            packet => Err(anyhow::anyhow!(
                "Expected {:?}, received {:?}",
                PacketType::Connect,
                packet_type(packet)
            )),
        }
    }

    pub async fn process_packet(
        packet: Packet,
        config: &BrokerConfig,
        state: &ServerState,
        client_id: &ClientID,
        username: &str,
        sender: &Sender<Packet>,
    ) -> Result<()> {
        info!("Processing: {:?}", packet);
        match packet {
            Packet::Connect(_) => Self::handle_invalid_packet(sender).await,
            Packet::ConnAck(_) => Self::handle_invalid_packet(sender).await,
            Packet::Publish(publish) => {
                Self::process_publish(publish, state, config.max_qos, username, sender).await
            }
            Packet::PubAck(_) => Self::handle_invalid_packet(sender).await,
            Packet::PubRec(_) => Self::handle_invalid_packet(sender).await,
            Packet::PubRel(_) => Self::handle_invalid_packet(sender).await,
            Packet::PubComp(_) => Self::handle_invalid_packet(sender).await,
            Packet::Subscribe(subscribe) => {
                Self::process_subscribe(
                    subscribe,
                    state,
                    config.max_qos,
                    client_id,
                    username,
                    sender,
                )
                .await
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

    async fn handle_invalid_packet(sender: &Sender<Packet>) -> Result<()> {
        sender
            .send(Packet::Disconnect(Disconnect {
                reason_code: DisconnectReasonCode::ProtocolError,
                properties: None,
            }))
            .await?;
        Err(anyhow::anyhow!("Received invalid packet"))
    }

    async fn process_disconnect(
        _: Disconnect,
        state: &ServerState,
        client_id: &ClientID,
    ) -> Result<()> {
        state.remove_client(client_id).await;
        Err(anyhow::anyhow!("Received DISCONNECT"))
    }

    async fn process_ping_req(sender: &Sender<Packet>) -> Result<()> {
        sender.send(Packet::PingResp).await?;
        Ok(())
    }

    async fn process_publish(
        publish: Publish,
        state: &ServerState,
        max_qos: QoS,
        username: &str,
        sender: &Sender<Packet>,
    ) -> Result<()> {
        if let Some(properties) = &publish.properties {
            if properties.topic_alias.is_some() {
                sender
                    .send(Packet::Disconnect(Disconnect {
                        reason_code: DisconnectReasonCode::ProtocolError,
                        properties: Some(DisconnectProperties {
                            reason_string: Some("Topic aliases are not supported".to_string()),
                            ..Default::default()
                        }),
                    }))
                    .await?;
                return Err(anyhow::anyhow!("Client specified unsupported topic alias"));
            }
        }

        if publish.topic.is_empty() || !valid_topic(&publish.topic) {
            sender
                .send(Packet::Disconnect(Disconnect {
                    reason_code: DisconnectReasonCode::TopicNameInvalid,
                    properties: None,
                }))
                .await?;
            return Err(anyhow::anyhow!("Client specified invalid topic name"));
        }

        if !state
            .auth_store
            .read()
            .await
            .can_user_publish(username, &publish.topic)?
        {
            sender
                .send(Packet::Disconnect(Disconnect {
                    reason_code: DisconnectReasonCode::NotAuthorized,
                    properties: None,
                }))
                .await?;
            return Err(anyhow::anyhow!("Unauthorized"));
        }

        if publish.retain {
            sender
                .send(Packet::Disconnect(Disconnect {
                    reason_code: DisconnectReasonCode::RetainNotSupported,
                    properties: None,
                }))
                .await?;
            return Err(anyhow::anyhow!("Client specified unsupported RETAIN flag"));
        }

        if publish.qos > max_qos {
            sender
                .send(Packet::Disconnect(Disconnect {
                    reason_code: DisconnectReasonCode::QoSNotSupported,
                    properties: Some(DisconnectProperties {
                        reason_string: Some(format!("QoS cannot exceed {:?}", max_qos)),
                        ..Default::default()
                    }),
                }))
                .await?;
            return Err(anyhow::anyhow!(
                "Client specified QoS above the specified maximum"
            ));
        }

        if publish.dup {
            sender
                .send(Packet::Disconnect(Disconnect {
                    reason_code: DisconnectReasonCode::ProtocolError,
                    properties: Some(DisconnectProperties {
                        reason_string: Some("DUP flag must be set to 0".to_string()),
                        ..Default::default()
                    }),
                }))
                .await?;
            return Err(anyhow::anyhow!("Client specified unsupported DUP flag"));
        }

        let clients = state.clients.read().await;
        let senders = clients.iter().filter_map(|(client_id, client_info)| {
            client_info
                .subscriptions
                .iter()
                .any(|filter| matches(&publish.topic, filter))
                .then_some((client_id.clone(), client_info.sender.clone()))
        });
        for (client_id, sender) in senders {
            if let Err(e) = sender.send(Packet::Publish(publish.clone())).await {
                warn!("Failed to publish to {}: {}", client_id, e);
            }
        }

        Ok(())
    }

    async fn process_subscribe(
        subscribe: Subscribe,
        state: &ServerState,
        max_qos: QoS,
        client_id: &ClientID,
        username: &str,
        sender: &Sender<Packet>,
    ) -> Result<()> {
        if subscribe
            .filters
            .iter()
            .any(|filter| filter.path.starts_with("$share"))
        {
            sender
                .send(Packet::Disconnect(Disconnect {
                    reason_code: DisconnectReasonCode::SharedSubscriptionNotSupported,
                    properties: None,
                }))
                .await?;
            return Err(anyhow::anyhow!("Shared subscriptions are not supported"));
        }

        let mut return_codes = Vec::new();

        for filter in subscribe.filters {
            if !valid_filter(&filter.path) {
                return_codes.push(SubscribeReasonCode::TopicFilterInvalid);
            } else if !state
                .auth_store
                .read()
                .await
                .can_user_subscribe(username, &filter.path)?
            {
                return_codes.push(SubscribeReasonCode::NotAuthorized);
            } else {
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
        }

        sender
            .send(Packet::SubAck(SubAck::new(subscribe.pkid, return_codes)))
            .await?;

        Ok(())
    }

    async fn process_unsubscribe(
        unsubscribe: Unsubscribe,
        state: &ServerState,
        client_id: &ClientID,
        sender: &Sender<Packet>,
    ) -> Result<()> {
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

        Ok(())
    }
}
