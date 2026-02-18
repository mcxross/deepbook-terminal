use anyhow::{anyhow, Context, Result};
use dashmap::DashMap;
use futures::StreamExt;
use reqwest::StatusCode;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::{
    collections::{BTreeSet, HashMap},
    sync::OnceLock,
    time::Duration,
};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::{sync::mpsc, task::JoinHandle};

use super::types::{
    Depth, PushQuote, SecurityDepth, SecurityQuote, SubFlags, Trade, TradeDirection, TradeSession,
    TradeStatus,
};

const DEFAULT_REST_BASE_URL: &str = "https://api.surflux.dev";
const DEFAULT_STREAM_BASE_URL: &str = "https://flux.surflux.dev";
const DEFAULT_SCALE_DECIMALS: u32 = 6;
const STREAM_KEY_SUFFIX: &str = "_STREAM_API_KEY";
const REST_KEY_SUFFIX: &str = "_REST_API_KEY";
const ALT_REST_KEY_SUFFIX: &str = "_API_KEY";
const REST_KEY_ENV: &str = "SURFLUX_REST_API_KEY";

type QuoteSnapshot = SecurityQuote;
type OrderBookSnapshot = SecurityDepth;

#[derive(Debug, Clone)]
pub struct StreamDepthLevel {
    pub position: i32,
    pub price: Option<Decimal>,
    pub volume: i64,
    pub order_num: i64,
}

#[derive(Debug, Clone, Default)]
pub struct StreamDepth {
    pub asks: Vec<StreamDepthLevel>,
    pub bids: Vec<StreamDepthLevel>,
}

#[derive(Debug, Clone)]
pub enum PushEventDetail {
    Quote(PushQuote),
    Depth(StreamDepth),
    Trades(Vec<Trade>),
}

#[derive(Debug, Clone)]
pub struct PushEvent {
    pub symbol: String,
    pub detail: PushEventDetail,
}

#[derive(Debug, Clone)]
pub struct WatchlistPoolEntry {
    pub symbol: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct WatchlistPoolGroup {
    pub id: u64,
    pub name: String,
    pub pools: Vec<WatchlistPoolEntry>,
}

#[derive(Debug, Clone)]
pub struct PoolMetadata {
    pub pool_name: String,
    pub pool_id: String,
    pub base_asset_symbol: String,
    pub quote_asset_symbol: String,
    pub is_margin: bool,
    pub base_asset_decimals: u32,
    pub quote_asset_decimals: u32,
    pub min_size: i64,
    pub lot_size: i64,
    pub tick_size: i64,
}

impl PoolMetadata {
    fn fallback(pool_name: &str) -> Self {
        let (base, quote) = parse_pool_name(pool_name);
        Self {
            pool_name: pool_name.to_string(),
            pool_id: String::new(),
            base_asset_symbol: base.to_string(),
            quote_asset_symbol: quote.to_string(),
            is_margin: looks_like_margin_pool_name(pool_name),
            base_asset_decimals: DEFAULT_SCALE_DECIMALS,
            quote_asset_decimals: DEFAULT_SCALE_DECIMALS,
            min_size: 0,
            lot_size: 0,
            tick_size: 0,
        }
    }

    pub fn display_name(&self) -> String {
        format!("{}/{}", self.base_asset_symbol, self.quote_asset_symbol)
    }
}

pub static QUOTE_CTX: OnceLock<QuoteContext> = OnceLock::new();

pub struct QuoteContext {
    client: reqwest::Client,
    rest_base_url: String,
    stream_base_url: String,
    stream_tx: mpsc::UnboundedSender<PushEvent>,
    stream_handles: DashMap<String, JoinHandle<()>>,
    pool_keys: DashMap<String, String>,
    rest_pool_keys: DashMap<String, String>,
    rest_default_key: Option<String>,
    pool_meta: DashMap<String, PoolMetadata>,
}

impl QuoteContext {
    fn new(stream_tx: mpsc::UnboundedSender<PushEvent>) -> Result<Self> {
        let rest_base_url = std::env::var("SURFLUX_API_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_REST_BASE_URL.to_string());
        let stream_base_url = std::env::var("SURFLUX_STREAM_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_STREAM_BASE_URL.to_string());

        let pool_key_map = discover_pool_keys(STREAM_KEY_SUFFIX);
        if pool_key_map.is_empty() {
            return Err(anyhow!(
                "No stream keys found. Set BASE_QUOTE_STREAM_API_KEY (for example: SUI_USDC_STREAM_API_KEY)."
            ));
        }
        let mut rest_pool_key_map = discover_pool_keys(REST_KEY_SUFFIX);
        for (symbol, key) in discover_pool_keys(ALT_REST_KEY_SUFFIX) {
            rest_pool_key_map.entry(symbol).or_insert(key);
        }
        let rest_default_key = std::env::var(REST_KEY_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty());

        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .build()
            .context("failed to build HTTP client")?;

        let ctx = Self {
            client,
            rest_base_url,
            stream_base_url,
            stream_tx,
            stream_handles: DashMap::new(),
            pool_keys: DashMap::new(),
            rest_pool_keys: DashMap::new(),
            rest_default_key,
            pool_meta: DashMap::new(),
        };

        for (pool_name, api_key) in pool_key_map {
            ctx.pool_keys.insert(pool_name.clone(), api_key);
            ctx.pool_meta
                .insert(pool_name.clone(), PoolMetadata::fallback(&pool_name));
        }
        for (pool_name, api_key) in rest_pool_key_map {
            ctx.rest_pool_keys.insert(pool_name, api_key);
        }

        Ok(ctx)
    }

    async fn bootstrap(&self) {
        let mut unique_keys: BTreeSet<String> = self
            .rest_pool_keys
            .iter()
            .map(|entry| entry.value().clone())
            .collect();
        if let Some(default_key) = &self.rest_default_key {
            unique_keys.insert(default_key.clone());
        }

        if unique_keys.is_empty() {
            tracing::warn!(
                "No REST API key configured; set SURFLUX_REST_API_KEY or BASE_QUOTE_REST_API_KEY for full REST features"
            );
            return;
        }

        for api_key in unique_keys {
            match self.fetch_pools_for_key(&api_key).await {
                Ok(pools) => {
                    for pool in pools {
                        if self.pool_keys.contains_key(&pool.pool_name) {
                            let metadata: PoolMetadata = pool.into();
                            self.pool_meta.insert(metadata.pool_name.clone(), metadata);
                        }
                    }
                }
                Err(err) => {
                    tracing::warn!("failed to fetch pool metadata: {err}");
                }
            }
        }

        let loaded = self.pool_meta.len();
        tracing::info!("Surflux initialized with {} configured pools", loaded);
    }

    pub fn configured_symbols(&self) -> Vec<String> {
        let mut symbols: Vec<String> = self
            .pool_keys
            .iter()
            .map(|entry| entry.key().clone())
            .collect();
        symbols.sort_unstable();
        symbols
    }

    pub fn base_asset_decimals(&self, symbol: &str) -> u32 {
        let symbol = normalize_symbol(symbol);
        self.pool_meta
            .get(&symbol)
            .map_or(DEFAULT_SCALE_DECIMALS, |meta| meta.base_asset_decimals)
    }

    pub async fn list_pools(&self) -> Result<Vec<PoolMetadata>> {
        let mut out = Vec::new();
        for symbol in self.configured_symbols() {
            out.push(self.ensure_pool_meta(&symbol).await?);
        }
        Ok(out)
    }

    pub async fn quote(&self, symbols: &[String]) -> Result<Vec<QuoteSnapshot>> {
        let mut quotes = Vec::with_capacity(symbols.len());
        for symbol in symbols {
            let symbol = normalize_symbol(symbol);
            match self.quote_for_symbol(&symbol).await {
                Ok(quote) => quotes.push(quote),
                Err(err) => tracing::warn!("failed to fetch quote for {symbol}: {err}"),
            }
        }
        Ok(quotes)
    }

    pub async fn watchlist(&self) -> Result<Vec<WatchlistPoolGroup>> {
        let pools = self.list_pools().await?;
        let mut spot_entries = Vec::new();
        let mut margin_entries = Vec::new();

        for pool in pools {
            let entry = WatchlistPoolEntry {
                symbol: pool.pool_name.clone(),
                name: pool.display_name(),
            };
            if pool.is_margin {
                margin_entries.push(entry);
            } else {
                spot_entries.push(entry);
            }
        }

        Ok(vec![
            WatchlistPoolGroup {
                id: 1,
                name: "Spot Pools".to_string(),
                pools: spot_entries,
            },
            WatchlistPoolGroup {
                id: 2,
                name: "Margin Pools".to_string(),
                pools: margin_entries,
            },
        ])
    }

    pub async fn trades(&self, symbol: &str, count: usize) -> Result<Vec<Trade>> {
        let symbol = normalize_symbol(symbol);
        let meta = self.ensure_pool_meta(&symbol).await?;
        let rows = self.fetch_trades(&symbol, count).await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let timestamp = parse_datetime(&row.timestamp)
                .or_else(|_| timestamp_from_millis(row.checkpoint_timestamp_ms))
                .unwrap_or_else(|_| OffsetDateTime::now_utc());

            out.push(Trade {
                price: scale_i64(row.price, meta.quote_asset_decimals),
                volume: row.base_quantity,
                timestamp,
                trade_type: String::new(),
                direction: if row.taker_is_bid {
                    TradeDirection::Up
                } else {
                    TradeDirection::Down
                },
                trade_session: TradeSession::Intraday,
            });
        }

        Ok(out)
    }

    pub async fn depth(&self, symbol: &str, limit: usize) -> Result<OrderBookSnapshot> {
        let symbol = normalize_symbol(symbol);
        let meta = self.ensure_pool_meta(&symbol).await?;
        let payload = self.fetch_depth(&symbol, limit).await?;

        let asks = payload
            .asks
            .into_iter()
            .enumerate()
            .map(|(idx, level)| Depth {
                position: i32::try_from(idx + 1).unwrap_or(i32::MAX),
                price: Some(scale_i64(level.price, meta.quote_asset_decimals)),
                volume: level.total_quantity,
                order_num: level.order_count,
            })
            .collect();

        let bids = payload
            .bids
            .into_iter()
            .enumerate()
            .map(|(idx, level)| Depth {
                position: i32::try_from(idx + 1).unwrap_or(i32::MAX),
                price: Some(scale_i64(level.price, meta.quote_asset_decimals)),
                volume: level.total_quantity,
                order_num: level.order_count,
            })
            .collect();

        Ok(OrderBookSnapshot { asks, bids })
    }

    pub async fn ohlcv(
        &self,
        symbol: &str,
        timeframe: &str,
        limit: usize,
    ) -> Result<Vec<OhlcvCandle>> {
        let symbol = normalize_symbol(symbol);
        let meta = self.ensure_pool_meta(&symbol).await?;
        let rows = self.fetch_ohlcv(&symbol, timeframe, limit).await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(OhlcvCandle {
                timestamp: parse_datetime(&row.timestamp)
                    .or_else(|_| timestamp_from_millis(None))
                    .unwrap_or_else(|_| OffsetDateTime::now_utc()),
                open: scale_i64(row.open, meta.quote_asset_decimals),
                high: scale_i64(row.high, meta.quote_asset_decimals),
                low: scale_i64(row.low, meta.quote_asset_decimals),
                close: scale_i64(row.close, meta.quote_asset_decimals),
                volume_base: row.volume_base.parse().unwrap_or(Decimal::ZERO),
                volume_quote: row.volume_quote.parse().unwrap_or(Decimal::ZERO),
                trade_count: row.trade_count,
            });
        }

        Ok(out)
    }

    pub async fn subscribe(&self, symbols: &[String], _sub_types: SubFlags) -> Result<()> {
        for symbol in symbols {
            let symbol = normalize_symbol(symbol);

            if self
                .stream_handles
                .get(&symbol)
                .is_some_and(|entry| !entry.value().is_finished())
            {
                continue;
            }

            let api_key = match self.stream_api_key(&symbol) {
                Ok(key) => key,
                Err(err) => {
                    tracing::warn!("Skipping stream subscription for {symbol}: {err}");
                    continue;
                }
            };

            let meta = self.ensure_pool_meta(&symbol).await?;
            let client = self.client.clone();
            let stream_base_url = self.stream_base_url.clone();
            let tx = self.stream_tx.clone();
            let symbol_clone = symbol.clone();

            let handle = tokio::spawn(async move {
                run_pool_stream(client, stream_base_url, symbol_clone, api_key, meta, tx).await;
            });

            self.stream_handles.insert(symbol, handle);
        }

        Ok(())
    }

    async fn quote_for_symbol(&self, symbol: &str) -> Result<QuoteSnapshot> {
        let symbol = normalize_symbol(symbol);
        let meta = self.ensure_pool_meta(&symbol).await?;

        let trades = self.fetch_trades(&symbol, 1).await.unwrap_or_default();
        let candles = self.fetch_ohlcv(&symbol, "1d", 2).await.unwrap_or_default();

        let last_trade = trades.first();
        let latest_candle = candles.last();
        let previous_candle = if candles.len() > 1 {
            candles.get(candles.len() - 2)
        } else {
            None
        };

        let fallback_price = latest_candle.map_or(Decimal::ZERO, |c| {
            scale_i64(c.close, meta.quote_asset_decimals)
        });

        let last_done = last_trade.map_or(fallback_price, |t| {
            scale_i64(t.price, meta.quote_asset_decimals)
        });

        let open =
            latest_candle.map_or(last_done, |c| scale_i64(c.open, meta.quote_asset_decimals));
        let high =
            latest_candle.map_or(last_done, |c| scale_i64(c.high, meta.quote_asset_decimals));
        let low = latest_candle.map_or(last_done, |c| scale_i64(c.low, meta.quote_asset_decimals));

        let prev_close =
            previous_candle.map_or(open, |c| scale_i64(c.close, meta.quote_asset_decimals));

        let timestamp = last_trade
            .and_then(|t| parse_datetime(&t.timestamp).ok())
            .or_else(|| latest_candle.and_then(|c| parse_datetime(&c.timestamp).ok()))
            .unwrap_or_else(OffsetDateTime::now_utc);

        let volume = latest_candle
            .and_then(|c| decimal_to_i64(c.volume_base.parse().ok()))
            .unwrap_or_default();

        let quote_volume = latest_candle
            .and_then(|c| c.volume_quote.parse().ok())
            .unwrap_or(Decimal::ZERO);

        Ok(QuoteSnapshot {
            symbol,
            last_done,
            prev_close,
            open,
            high,
            low,
            timestamp,
            volume,
            turnover: quote_volume,
            trade_status: TradeStatus::Normal,
            pre_market_quote: None,
            post_market_quote: None,
            overnight_quote: None,
        })
    }

    async fn ensure_pool_meta(&self, symbol: &str) -> Result<PoolMetadata> {
        let symbol = normalize_symbol(symbol);
        if let Some(meta) = self.pool_meta.get(&symbol) {
            return Ok(meta.value().clone());
        }

        let api_key = self.rest_api_key(&symbol)?;
        let pools = self.fetch_pools_for_key(&api_key).await?;
        for pool in pools {
            let metadata: PoolMetadata = pool.into();
            self.pool_meta
                .insert(metadata.pool_name.clone(), metadata.clone());
        }

        self.pool_meta
            .get(&symbol)
            .map(|meta| meta.value().clone())
            .ok_or_else(|| anyhow!("pool metadata not found for {symbol}"))
    }

    fn stream_api_key(&self, symbol: &str) -> Result<String> {
        let symbol = normalize_symbol(symbol);
        self.pool_keys
            .get(&symbol)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| anyhow!("missing {symbol}{STREAM_KEY_SUFFIX}"))
    }

    fn rest_api_key(&self, symbol: &str) -> Result<String> {
        let symbol = normalize_symbol(symbol);
        if let Some(api_key) = self.rest_pool_keys.get(&symbol) {
            return Ok(api_key.value().clone());
        }
        if let Some(api_key) = &self.rest_default_key {
            return Ok(api_key.clone());
        }
        Err(anyhow!(
            "missing REST API key for {symbol}; set {REST_KEY_ENV} or {symbol}{REST_KEY_SUFFIX}"
        ))
    }

    async fn fetch_pools_for_key(&self, api_key: &str) -> Result<Vec<PoolDto>> {
        let url = format!(
            "{}/deepbook/get_pools",
            self.rest_base_url.trim_end_matches('/')
        );

        let response = self
            .client
            .get(url)
            .query(&[("api-key", api_key)])
            .send()
            .await
            .context("failed to request pool list")?;

        ensure_success(&response, "get_pools")?;
        response
            .json::<Vec<PoolDto>>()
            .await
            .context("failed to decode get_pools response")
    }

    async fn fetch_trades(&self, symbol: &str, count: usize) -> Result<Vec<TradeDto>> {
        let api_key = self.rest_api_key(symbol)?;
        let limit_value = count.to_string();
        let url = format!(
            "{}/deepbook/{symbol}/trades",
            self.rest_base_url.trim_end_matches('/')
        );

        let response = self
            .client
            .get(url)
            .query(&[
                ("api-key", api_key.as_str()),
                ("limit", limit_value.as_str()),
            ])
            .send()
            .await
            .with_context(|| format!("failed to request trades for {symbol}"))?;

        ensure_success(&response, "trades")?;
        response
            .json::<Vec<TradeDto>>()
            .await
            .with_context(|| format!("failed to decode trades for {symbol}"))
    }

    async fn fetch_depth(&self, symbol: &str, limit: usize) -> Result<DepthDto> {
        let api_key = self.rest_api_key(symbol)?;
        let limit_value = limit.to_string();
        let url = format!(
            "{}/deepbook/{symbol}/order-book-depth",
            self.rest_base_url.trim_end_matches('/')
        );

        let response = self
            .client
            .get(url)
            .query(&[
                ("api-key", api_key.as_str()),
                ("limit", limit_value.as_str()),
            ])
            .send()
            .await
            .with_context(|| format!("failed to request depth for {symbol}"))?;

        ensure_success(&response, "order-book-depth")?;
        response
            .json::<DepthDto>()
            .await
            .with_context(|| format!("failed to decode depth for {symbol}"))
    }

    async fn fetch_ohlcv(
        &self,
        symbol: &str,
        timeframe: &str,
        limit: usize,
    ) -> Result<Vec<OhlcvDto>> {
        let api_key = self.rest_api_key(symbol)?;
        let limit_value = limit.to_string();
        let url = format!(
            "{}/deepbook/{symbol}/ohlcv/{timeframe}",
            self.rest_base_url.trim_end_matches('/')
        );

        let response = self
            .client
            .get(url)
            .query(&[
                ("api-key", api_key.as_str()),
                ("limit", limit_value.as_str()),
            ])
            .send()
            .await
            .with_context(|| format!("failed to request OHLCV for {symbol}"))?;

        ensure_success(&response, "ohlcv")?;
        response
            .json::<Vec<OhlcvDto>>()
            .await
            .with_context(|| format!("failed to decode OHLCV for {symbol}"))
    }
}

#[derive(Debug, Clone)]
pub struct OhlcvCandle {
    pub timestamp: OffsetDateTime,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
    pub volume_base: Decimal,
    pub volume_quote: Decimal,
    pub trade_count: u64,
}

#[derive(Debug, Deserialize)]
struct PoolDto {
    pool_id: String,
    pool_name: String,
    base_asset_symbol: String,
    quote_asset_symbol: String,
    #[serde(default)]
    is_margin: Option<serde_json::Value>,
    #[serde(default)]
    pool_type: Option<serde_json::Value>,
    #[serde(default)]
    market_type: Option<serde_json::Value>,
    #[serde(default, rename = "type")]
    type_field: Option<serde_json::Value>,
    #[serde(default)]
    category: Option<serde_json::Value>,
    #[serde(default)]
    kind: Option<serde_json::Value>,
    base_asset_decimals: u32,
    quote_asset_decimals: u32,
    min_size: i64,
    lot_size: i64,
    tick_size: i64,
}

impl PoolDto {
    fn is_margin_pool(&self) -> bool {
        if self.is_margin.as_ref().is_some_and(value_truthy) {
            return true;
        }

        for field in [
            self.pool_type.as_ref(),
            self.market_type.as_ref(),
            self.type_field.as_ref(),
            self.category.as_ref(),
            self.kind.as_ref(),
        ] {
            if field.is_some_and(value_contains_margin) {
                return true;
            }
        }

        looks_like_margin_pool_name(&self.pool_name)
    }
}

impl From<PoolDto> for PoolMetadata {
    fn from(value: PoolDto) -> Self {
        let is_margin = value.is_margin_pool();
        Self {
            pool_name: normalize_symbol(&value.pool_name),
            pool_id: value.pool_id,
            base_asset_symbol: value.base_asset_symbol,
            quote_asset_symbol: value.quote_asset_symbol,
            is_margin,
            base_asset_decimals: value.base_asset_decimals,
            quote_asset_decimals: value.quote_asset_decimals,
            min_size: value.min_size,
            lot_size: value.lot_size,
            tick_size: value.tick_size,
        }
    }
}

#[derive(Debug, Deserialize)]
struct TradeDto {
    price: i64,
    base_quantity: i64,
    #[serde(rename = "quote_quantity")]
    _quote_quantity: i64,
    #[serde(default)]
    taker_is_bid: bool,
    timestamp: String,
    #[serde(default)]
    checkpoint_timestamp_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct DepthLevelDto {
    price: i64,
    total_quantity: i64,
    order_count: i64,
}

#[derive(Debug, Deserialize)]
struct DepthDto {
    #[allow(dead_code)]
    pool_id: String,
    asks: Vec<DepthLevelDto>,
    bids: Vec<DepthLevelDto>,
}

#[derive(Debug, Deserialize)]
struct OhlcvDto {
    #[serde(default)]
    timestamp: String,
    #[serde(default, deserialize_with = "de_i64_from_number_or_string")]
    open: i64,
    #[serde(default, deserialize_with = "de_i64_from_number_or_string")]
    high: i64,
    #[serde(default, deserialize_with = "de_i64_from_number_or_string")]
    low: i64,
    #[serde(default, deserialize_with = "de_i64_from_number_or_string")]
    close: i64,
    #[serde(default, deserialize_with = "de_string_from_number_or_string")]
    volume_base: String,
    #[serde(default, deserialize_with = "de_string_from_number_or_string")]
    volume_quote: String,
    #[serde(default, deserialize_with = "de_u64_from_number_or_string")]
    trade_count: u64,
}

#[derive(Debug, Deserialize)]
struct SseEnvelope {
    #[serde(rename = "type")]
    event_type: String,
    #[allow(dead_code)]
    checkpoint_id: Option<i64>,
    timestamp_ms: Option<i64>,
    data: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct SseTradeData {
    price: i64,
    base_quantity: i64,
    quote_quantity: i64,
    #[serde(default)]
    taker_is_bid: bool,
    #[serde(default)]
    checkpoint_timestamp_ms: Option<i64>,
    #[serde(default)]
    onchain_timestamp: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct SseDepthData {
    asks: Vec<DepthLevelDto>,
    bids: Vec<DepthLevelDto>,
}

pub async fn init_contexts() -> Result<impl tokio_stream::Stream<Item = PushEvent> + Send + Unpin> {
    let (stream_tx, stream_rx) = mpsc::unbounded_channel();
    let quote_ctx = QuoteContext::new(stream_tx)?;
    quote_ctx.bootstrap().await;

    QUOTE_CTX
        .set(quote_ctx)
        .map_err(|_| anyhow!("QuoteContext already initialized"))?;

    Ok(tokio_stream::wrappers::UnboundedReceiverStream::new(
        stream_rx,
    ))
}

pub fn quote() -> &'static QuoteContext {
    QUOTE_CTX
        .get()
        .expect("QuoteContext not initialized, please call init_contexts() first")
}

pub fn print_config_guide() {
    eprintln!("Configuration Error: Missing Surflux stream API keys");
    eprintln!();
    eprintln!("Stream key (required, one per pool):");
    eprintln!("  BASE_QUOTE_STREAM_API_KEY=<api_key>");
    eprintln!("Example:");
    eprintln!("  SUI_USDC_STREAM_API_KEY=<stream_key>");
    eprintln!();
    eprintln!("REST key (required for OHLCV/trades/depth metadata):");
    eprintln!("  SURFLUX_REST_API_KEY=<rest_key>");
    eprintln!("Optional per-pool REST override:");
    eprintln!("  BASE_QUOTE_REST_API_KEY=<rest_key>");
    eprintln!("Example:");
    eprintln!("  SUI_USDC_REST_API_KEY=<rest_key>");
    eprintln!();
    eprintln!("Optional overrides:");
    eprintln!("  SURFLUX_API_BASE_URL=https://api.surflux.dev");
    eprintln!("  SURFLUX_STREAM_BASE_URL=https://flux.surflux.dev");
}

async fn run_pool_stream(
    client: reqwest::Client,
    stream_base_url: String,
    symbol: String,
    api_key: String,
    meta: PoolMetadata,
    tx: mpsc::UnboundedSender<PushEvent>,
) {
    let endpoint = format!(
        "{}/deepbook/{}/live-trades",
        stream_base_url.trim_end_matches('/'),
        symbol
    );

    let mut last_id = "$".to_string();

    loop {
        let response = client
            .get(&endpoint)
            .query(&[("api-key", api_key.as_str()), ("last-id", last_id.as_str())])
            .header("accept", "text/event-stream")
            .send()
            .await;

        let response = match response {
            Ok(resp) => resp,
            Err(err) => {
                tracing::warn!("SSE connect failed for {symbol}: {err}");
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };

        if response.status() != StatusCode::OK {
            tracing::warn!(
                "SSE stream returned {} for {symbol}",
                response.status().as_u16()
            );
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }

        match consume_sse_stream(response.bytes_stream(), &symbol, &meta, &tx, &mut last_id).await {
            Ok(()) => {
                tracing::warn!("SSE stream ended for {symbol}, reconnecting");
            }
            Err(err) => {
                tracing::warn!("SSE stream error for {symbol}: {err}");
            }
        }

        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn consume_sse_stream<S>(
    mut stream: S,
    symbol: &str,
    meta: &PoolMetadata,
    tx: &mpsc::UnboundedSender<PushEvent>,
    last_id: &mut String,
) -> Result<()>
where
    S: futures::Stream<Item = std::result::Result<bytes::Bytes, reqwest::Error>> + Unpin,
{
    let mut pending = String::new();
    let mut event_data = String::new();
    let mut event_id: Option<String> = None;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk
            .map_err(|err| anyhow!(err))
            .context("failed reading SSE chunk")?;
        pending.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = pending.find('\n') {
            let mut line = pending[..pos].to_string();
            pending.drain(..=pos);

            if line.ends_with('\r') {
                line.pop();
            }

            if line.is_empty() {
                if !event_data.is_empty() {
                    if let Ok(event) = serde_json::from_str::<SseEnvelope>(&event_data) {
                        process_sse_event(symbol, meta, event, tx);
                    }
                    event_data.clear();
                }

                if let Some(id) = event_id.take() {
                    *last_id = id;
                }
                continue;
            }

            if let Some(data) = line.strip_prefix("data:") {
                if !event_data.is_empty() {
                    event_data.push('\n');
                }
                event_data.push_str(data.trim_start());
            } else if let Some(id) = line.strip_prefix("id:") {
                event_id = Some(id.trim().to_string());
            }
        }
    }

    Ok(())
}

fn process_sse_event(
    symbol: &str,
    meta: &PoolMetadata,
    envelope: SseEnvelope,
    tx: &mpsc::UnboundedSender<PushEvent>,
) {
    match envelope.event_type.as_str() {
        "deepbook_live_trades" => {
            let Ok(data) = serde_json::from_value::<SseTradeData>(envelope.data) else {
                return;
            };

            let timestamp = timestamp_from_millis(
                data.checkpoint_timestamp_ms
                    .or(data.onchain_timestamp)
                    .or(envelope.timestamp_ms),
            )
            .unwrap_or_else(|_| OffsetDateTime::now_utc());

            let price = scale_i64(data.price, meta.quote_asset_decimals);
            let quote_volume = scale_i64(data.quote_quantity, meta.quote_asset_decimals);

            let quote = PushQuote {
                last_done: price,
                open: price,
                high: price,
                low: price,
                timestamp,
                volume: data.base_quantity,
                turnover: quote_volume,
                trade_status: TradeStatus::Normal,
                trade_session: TradeSession::Intraday,
                current_volume: data.base_quantity,
                current_turnover: quote_volume,
            };

            _ = tx.send(PushEvent {
                symbol: symbol.to_string(),
                detail: PushEventDetail::Quote(quote),
            });

            let trade = Trade {
                price,
                volume: data.base_quantity,
                timestamp,
                trade_type: String::new(),
                direction: if data.taker_is_bid {
                    TradeDirection::Up
                } else {
                    TradeDirection::Down
                },
                trade_session: TradeSession::Intraday,
            };

            _ = tx.send(PushEvent {
                symbol: symbol.to_string(),
                detail: PushEventDetail::Trades(vec![trade]),
            });
        }
        "deepbook_order_book_depth" => {
            let Ok(data) = serde_json::from_value::<SseDepthData>(envelope.data) else {
                return;
            };

            let asks = data
                .asks
                .into_iter()
                .enumerate()
                .map(|(idx, level)| StreamDepthLevel {
                    position: i32::try_from(idx + 1).unwrap_or(i32::MAX),
                    price: Some(scale_i64(level.price, meta.quote_asset_decimals)),
                    volume: level.total_quantity,
                    order_num: level.order_count,
                })
                .collect();

            let bids = data
                .bids
                .into_iter()
                .enumerate()
                .map(|(idx, level)| StreamDepthLevel {
                    position: i32::try_from(idx + 1).unwrap_or(i32::MAX),
                    price: Some(scale_i64(level.price, meta.quote_asset_decimals)),
                    volume: level.total_quantity,
                    order_num: level.order_count,
                })
                .collect();

            _ = tx.send(PushEvent {
                symbol: symbol.to_string(),
                detail: PushEventDetail::Depth(StreamDepth { asks, bids }),
            });
        }
        _ => {}
    }
}

fn discover_pool_keys(suffix: &str) -> HashMap<String, String> {
    let mut keys = HashMap::new();

    for (name, value) in std::env::vars() {
        let Some(prefix) = name.strip_suffix(suffix) else {
            continue;
        };

        let mut parts = prefix.split('_');
        let (Some(base), Some(quote), None) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };

        if value.trim().is_empty() {
            continue;
        }

        let symbol = normalize_symbol(&format!("{base}_{quote}"));
        keys.insert(symbol, value);
    }

    keys
}

fn de_i64_from_number_or_string<'de, D>(deserializer: D) -> std::result::Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Number(num) => num
            .as_i64()
            .ok_or_else(|| D::Error::custom("expected integer number")),
        serde_json::Value::String(s) => s
            .trim()
            .parse::<i64>()
            .map_err(|err| D::Error::custom(format!("invalid i64 string: {err}"))),
        serde_json::Value::Null => Ok(0),
        other => Err(D::Error::custom(format!(
            "expected number or string, got {other}"
        ))),
    }
}

fn de_u64_from_number_or_string<'de, D>(deserializer: D) -> std::result::Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Number(num) => num
            .as_u64()
            .or_else(|| num.as_i64().and_then(|v| u64::try_from(v).ok()))
            .ok_or_else(|| D::Error::custom("expected non-negative integer number")),
        serde_json::Value::String(s) => s
            .trim()
            .parse::<u64>()
            .map_err(|err| D::Error::custom(format!("invalid u64 string: {err}"))),
        serde_json::Value::Null => Ok(0),
        other => Err(D::Error::custom(format!(
            "expected number or string, got {other}"
        ))),
    }
}

fn de_string_from_number_or_string<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::String(s) => s,
        serde_json::Value::Number(num) => num.to_string(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    })
}

fn value_truthy(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Bool(v) => *v,
        serde_json::Value::Number(v) => v.as_i64().is_some_and(|n| n != 0),
        serde_json::Value::String(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "true" | "1" | "yes" | "y" | "margin"
        ),
        _ => false,
    }
}

fn value_contains_margin(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(v) => v.trim().to_ascii_lowercase().contains("margin"),
        serde_json::Value::Object(map) => map.values().any(value_contains_margin),
        serde_json::Value::Array(list) => list.iter().any(value_contains_margin),
        _ => false,
    }
}

fn looks_like_margin_pool_name(pool_name: &str) -> bool {
    let normalized = normalize_symbol(pool_name);
    normalized.contains("MARGIN") || normalized.ends_with("_M")
}

fn parse_pool_name(pool_name: &str) -> (&str, &str) {
    let mut parts = pool_name.split('_');
    match (parts.next(), parts.next()) {
        (Some(base), Some(quote)) => (base, quote),
        _ => (pool_name, "QUOTE"),
    }
}

fn normalize_symbol(symbol: &str) -> String {
    symbol.trim().to_ascii_uppercase().replace('.', "_")
}

fn scale_i64(value: i64, decimals: u32) -> Decimal {
    Decimal::from_i128_with_scale(i128::from(value), decimals)
}

fn decimal_to_i64(value: Option<Decimal>) -> Option<i64> {
    value.and_then(|v| v.round_dp(0).to_string().parse::<i64>().ok())
}

fn parse_datetime(value: &str) -> Result<OffsetDateTime> {
    OffsetDateTime::parse(value, &Rfc3339).map_err(|err| anyhow!(err))
}

fn timestamp_from_millis(timestamp_ms: Option<i64>) -> Result<OffsetDateTime> {
    let ts = timestamp_ms.unwrap_or_else(|| OffsetDateTime::now_utc().unix_timestamp() * 1_000);
    let nanos = i128::from(ts) * 1_000_000;
    OffsetDateTime::from_unix_timestamp_nanos(nanos).map_err(|err| anyhow!(err))
}

fn ensure_success(response: &reqwest::Response, endpoint: &str) -> Result<()> {
    if response.status().is_success() {
        return Ok(());
    }

    Err(anyhow!(
        "surflux {} request failed: HTTP {}",
        endpoint,
        response.status().as_u16()
    ))
}
