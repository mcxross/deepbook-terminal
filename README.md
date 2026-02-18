# Surflux Terminal

A Rust TUI for monitoring DeepBook activities with live stream updates.

## Features

- Real-time DeepBook pool watchlist
- Live order-book depth and trades
- Pool search
- OHLCV candlestick charts

## Requirements

- macOS or Linux
- Surflux stream API key(s)
- Surflux REST API key (for OHLCV/trades/depth endpoints)

## Installation

```bash
curl -fsSL https://raw.githubusercontent.com/mcxross/deepbook-terminal/main/install | sh
```

## Configuration

`strike` reads configuration from process environment variables.

Set required keys directly:

```bash
export SUI_USDC_STREAM_API_KEY=...
export SURFLUX_REST_API_KEY=...
```

Required keys:

- Stream key(s): `BASE_QUOTE_STREAM_API_KEY` (for example `SUI_USDC_STREAM_API_KEY`)
- REST key: `SURFLUX_REST_API_KEY` (global) or `BASE_QUOTE_REST_API_KEY` per pool

Set one API key per pool using:

```bash
BASE_QUOTE_STREAM_API_KEY=your_pool_stream_key
```

Examples:

```bash
SUI_USDC_STREAM_API_KEY=...
DEEP_USDC_STREAM_API_KEY=...
WAL_USDC_STREAM_API_KEY=...
```

Set REST key separately (global):

```bash
SURFLUX_REST_API_KEY=...
```

Optional overrides:

```bash
SURFLUX_API_BASE_URL=https://api.surflux.dev
SURFLUX_STREAM_BASE_URL=https://flux.surflux.dev
LOG=error,strike=debug
```

## Run

```bash
cargo run
```

## Credits

This project is a heavy adaptation of the [Long Bridge Terminal](https://github.com/longbridge/longbridge-terminal) 
added with DeepBook-specific features and a new UI.
