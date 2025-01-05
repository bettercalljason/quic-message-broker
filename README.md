# Running with Cargo
```bash
# Server
cargo run --  --key "..\example\tls\key.der" --cert "..\example\tls\cert.der"

# Client
cargo run -- --ca "..\example\tls\cert.der"

```

Note that the server certificate must not be a CA.