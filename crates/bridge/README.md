# Polyglot Bridge

A WebSocket bridge that connects Expo/mobile clients to Polyglot-AI servers over QUIC.

## Usage

```bash
cargo build -p polyglot-bridge --release

# WS on 8787, QUIC to server on 4433
./target/release/polyglot-bridge --listen 0.0.0.0:8787 --server 127.0.0.1:4433
```

## Local mode

```bash
./target/release/polyglot-bridge --listen 0.0.0.0:8787 --mode local --local-bin polyglot-local
```

### Optional auth token

```bash
./target/release/polyglot-bridge --token <shared-token>
```

Mobile clients should connect to `ws://<host>:8787/ws?token=<shared-token>`.

## Notes

- The bridge forwards Polyglot `ClientMessage` and `ServerMessage` frames.
- JSON payloads are accepted as text frames, MessagePack as binary frames.
