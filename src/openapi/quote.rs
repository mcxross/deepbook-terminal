use crate::openapi::context::QUOTE_CTX;
use crate::openapi::types::Trade;
use anyhow::{anyhow, Result};
use rust_decimal::Decimal;

const DEFAULT_BASE_DECIMALS: u32 = 6;
pub async fn fetch_trades(symbol: &str, count: usize) -> Result<Vec<Trade>> {
    let ctx = QUOTE_CTX
        .get()
        .ok_or_else(|| anyhow!("QuoteContext not initialized"))?;
    let trades = ctx.trades(symbol, count).await?;
    Ok(trades)
}

pub fn base_asset_decimals(symbol: &str) -> u32 {
    QUOTE_CTX
        .get()
        .map_or(DEFAULT_BASE_DECIMALS, |ctx| ctx.base_asset_decimals(symbol))
}

pub fn normalize_base_volume_u64(symbol: &str, raw_volume: u64) -> Decimal {
    Decimal::from_i128_with_scale(i128::from(raw_volume), base_asset_decimals(symbol))
}

pub fn normalize_base_volume_i64(symbol: &str, raw_volume: i64) -> Decimal {
    Decimal::from_i128_with_scale(i128::from(raw_volume), base_asset_decimals(symbol))
}
