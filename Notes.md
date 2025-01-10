There are probably still some tasks:

server:
- (p1) change port to common MQTT, check the alpn thingy
- (p1) graceful close error code
- (p2) keep alive monitoring and QUIC keep-alive setting
- (p2) maybe send connack anyway on error, because the ALPN exposes me anyway...
- (p3) auth store with users read from file
- (p3) graceful shutdown, disconnecting clients (first MQTT disconnect then completely)
- (p3) last will action
- (p3) subscription ID's

========================================================================================

Features:
- security: Authentication and Authorization (auth store currently hard-coded)
- Pub/Sub
- security: Single stream
- security: Rejecting invalid connections (no connack)

Broker NOT implemented features (but handled):
- QoS > 0
- Shared Subscriptions
- Topic Alias
- Retain

Report:
- I think I could stay on a single stream and make sure to only accept one! That would be a "secure" decision and can be then discussed in my report. I think this would make it more realistic to complete the project in due time.
- Also, there is stuff to write about regarding mqtt security (e.g. see the spec in 5.4.2 where it talks about access control for PUB/SUB etc.)
- I didn't really look into QUIC configuration. What is the matter with 0RTT? What are the pitfalls? What about https://www.rfc-editor.org/rfc/rfc9308.html?

Demo:
- How about 1 client that publishes my CPU temp in fixed intervals and then I can show how I can subscribe/unsubscribe etc.