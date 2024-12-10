[package]
name = "quic-message-broker"
version = "0.1.0"
edition = "2021"

[features]
# Configure `tracing` to log events via `log` if no `tracing` subscriber exists.
log = ["tracing/log"]
# Enable rustls logging
#rustls-log = ["rustls?/logging"]

[dependencies]
anyhow = "1.0.93"
async-trait = "0.1.83"
bytes = "1.9.0"
clap = { version = "4.5.21", features = ["derive"] }
directories-next = "2.0.0"
mqttbytes = "0.6.0"
quinn = "0.11.6"
quinn-proto = "0.11.9"
rcgen = "0.13.1"
rustls = { version = "0.23.19", features = ["ring"] }
rustls-pemfile = "2.2.0"
tokio = { version = "1.41.1", features = ["full"] }
tracing = "0.1.41"
tracing-futures = "0.2.5"
tracing-subscriber = { version = "0.3.19", features = ["env-filter"] }
url = "2.5.4"

#[[bin]]
#name = "server"

#[[bin]]
#name = "client"
