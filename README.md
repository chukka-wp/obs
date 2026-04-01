# Chukka OBS

Standalone binary that bridges [Chukka](https://github.com/chukka-wp/chukka) match state to OBS Studio. Connects to chukka-cloud via WebSocket and serves HTML overlays to OBS browser sources on `localhost:4747`.

## Features

- Real-time match state subscription via WebSocket
- Serves scorebug, exclusion timer, and event animation overlays
- Auto-reconnect with exponential backoff
- Token-based authentication (OBS tokens from chukka-cloud)
- Zero configuration once connected — overlays update automatically

## Tech Stack

Rust

## Setup

```bash
cargo build --release
```

## Usage

```bash
./chukka-obs --cloud-url wss://chukka.app --token <obs-token>
```

Add browser sources in OBS pointing to `http://localhost:4747/scorebug`, etc.

## License

[Elastic License 2.0 (ELv2)](https://www.elastic.co/licensing/elastic-license)
