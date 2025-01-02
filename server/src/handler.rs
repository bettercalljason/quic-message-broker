use crate::state::ServerState;
use anyhow::Result;
use mqttbytes::{v5::*, QoS};
use myprotocol::ClientID;
use tracing::info;

pub struct BrokerConfig {
    pub max_qos: QoS,
}

pub struct PacketHandler;

impl PacketHandler {
    pub async fn process_first_packet(
        packet: Packet,
        config: &BrokerConfig,
        state: &mut ServerState,
    ) -> Result<(Option<ClientID>, Option<Packet>, bool)> {
        match packet {
            Packet::Connect(connect) => Self::process_connect(connect, config.max_qos, state),
            _ => Self::handle_invalid_packet().map(|(packet, disconnect)| (None, packet, disconnect)),
        }
    }

    pub async fn process_packet(
        packet: Packet,
        config: &BrokerConfig,
        state: &mut ServerState,
        client_id: &ClientID,
    ) -> Result<(Option<Packet>, bool)> {
        info!("Processing: {:?}", packet);
        match packet {
            Packet::Connect(_) => Self::handle_invalid_packet(),
            Packet::ConnAck(_) => Self::handle_invalid_packet(),
            Packet::Publish(publish) => Self::process_publish(publish),
            Packet::PubAck(_) => Self::handle_invalid_packet(),
            Packet::PubRec(_) => Self::handle_invalid_packet(),
            Packet::PubRel(_) => Self::handle_invalid_packet(),
            Packet::PubComp(_) => Self::handle_invalid_packet(),
            Packet::Subscribe(subscribe) => {
                Self::process_subscribe(subscribe, state, config.max_qos, client_id)
            }
            Packet::SubAck(_) => Self::handle_invalid_packet(),
            Packet::Unsubscribe(unsubscribe) => {
                Self::process_unsubscribe(unsubscribe, state, client_id)
            }
            Packet::UnsubAck(_) => Self::handle_invalid_packet(),
            Packet::PingReq => Self::process_ping_req(),
            Packet::PingResp => Self::handle_invalid_packet(),
            Packet::Disconnect(disconnect) => {
                Self::process_disconnect(disconnect, state, client_id)
            }
        }
    }

    fn process_connect(
        connect: Connect,
        max_qos: QoS,
        state: &mut ServerState,
    ) -> Result<(Option<ClientID>, Option<Packet>, bool)> {
        let client_id = ClientID::try_from(connect.client_id)?;
        if let Some(will) = &connect.last_will {
            if will.qos > max_qos {
                // Will QoS exceeds max allowed QoS; reject the connection
                let response = Packet::ConnAck(ConnAck {
                    session_present: false,
                    code: ConnectReturnCode::QoSNotSupported,
                    properties: None,
                });
                return Ok((Some(client_id), Some(response), true));
            }
        }

        state.add_client(&client_id);

        let response = Packet::ConnAck(ConnAck {
            session_present: false,
            code: ConnectReturnCode::Success,
            properties: None,
        });
        Ok((Some(client_id), Some(response), false))
    }

    fn handle_invalid_packet() -> Result<(Option<Packet>, bool)> {
        Ok((
            Some(Packet::Disconnect(Disconnect {
                reason_code: DisconnectReasonCode::ProtocolError,
                properties: None,
            })),
            true,
        ))
    }

    fn process_disconnect(
        _: Disconnect,
        state: &mut ServerState,
        client_id: &ClientID,
    ) -> Result<(Option<Packet>, bool)> {
        state.remove_client(client_id);
        Ok((None, true))
    }

    fn process_ping_req() -> Result<(Option<Packet>, bool)> {
        Ok((Some(Packet::PingResp), false))
    }

    fn process_publish(_: Publish) -> Result<(Option<Packet>, bool)> {
        todo!()
    }

    fn process_subscribe(
        subscribe: Subscribe,
        state: &mut ServerState,
        max_qos: QoS,
        client_id: &ClientID,
    ) -> Result<(Option<Packet>, bool)> {
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
            state.add_subscription(topic, client_id);
        }

        Ok((
            Some(Packet::SubAck(SubAck::new(subscribe.pkid, return_codes))),
            false,
        ))
    }

    fn process_unsubscribe(
        unsubscribe: Unsubscribe,
        state: &mut ServerState,
        client_id: &ClientID,
    ) -> Result<(Option<Packet>, bool)> {
        let mut reasons = Vec::new();

        for filter in unsubscribe.filters {
            let removed = state.remove_subscription(filter, client_id);
            reasons.push(match removed {
                true => UnsubAckReason::Success,
                false => UnsubAckReason::NoSubscriptionExisted,
            });
        }
        Ok((
            Some(Packet::UnsubAck(UnsubAck {
                pkid: unsubscribe.pkid,
                reasons,
                properties: None,
            })),
            false,
        ))
    }
}
