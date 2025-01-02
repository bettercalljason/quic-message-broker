use anyhow::Result;
use bytes::{BufMut, BytesMut};
use mqttbytes::{v5::*, PacketType};
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
        match self.read_patched(buffer, self.max_packet_size) {
            Ok(packet) => Ok(Some(packet)),
            Err(mqttbytes::Error::InsufficientBytes(_)) => Ok(None), // Partial data, wait for more
            Err(e) => Err(anyhow::anyhow!(e)),
        }
    }

    /// Reads a stream of bytes and extracts next MQTT packet out of it
    /// Patched version of mqttbytes::v5::read, because that one did not accept Disconnects without payload
    fn read_patched(
        &self,
        stream: &mut BytesMut,
        max_size: usize,
    ) -> Result<Packet, mqttbytes::Error> {
        let fixed_header = mqttbytes::check(stream.iter(), max_size)?;

        // Test with a stream with exactly the size to check border panics
        let packet = stream.split_to(fixed_header.frame_length());
        let packet_type = fixed_header.packet_type()?;

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
}
