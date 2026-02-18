use std::{collections::HashMap, sync::RwLock};

use crate::data::{Counter, Kline, KlineType, Klines};
use rust_decimal::Decimal;

pub static KLINES: std::sync::LazyLock<KlineStore> = std::sync::LazyLock::new(KlineStore::new);

type StoreKey = (Counter, KlineType);

#[derive(Debug)]
pub struct KlineStore {
    inner: RwLock<HashMap<StoreKey, (bool, Klines)>>,
}

impl KlineStore {
    fn new() -> Self {
        Self {
            inner: RwLock::default(),
        }
    }

    pub fn by_pagination(
        &self,
        counter: Counter,
        kline_type: KlineType,
        page: usize,
        page_size: usize,
    ) -> Klines {
        let store = self.inner.read().expect("poison");
        let Some((has_more, entries)) = store.get(&(counter.clone(), kline_type)) else {
            crate::app::RT.get().unwrap().spawn(Self::request(
                counter,
                kline_type,
                0,
                (page + 1) * page_size,
            ));
            return Klines::default();
        };

        let tmp: Klines;
        let results = if let Some(offset) = entries.len().checked_sub(page * page_size) {
            &entries[offset.saturating_sub(page_size)..offset]
        } else {
            tmp = vec![];
            &tmp
        };

        if *has_more && results.len() < page_size {
            crate::app::RT.get().unwrap().spawn(Self::request(
                counter,
                kline_type,
                entries.first().map(|e| e.timestamp).unwrap_or_default(),
                page_size,
            ));
        }
        results.to_vec()
    }

    pub fn clear(&self) {
        let mut store = self.inner.write().expect("poison");
        store.clear();
    }
    pub fn update(&self, counter: Counter, kline_type: KlineType, data: Klines, more: bool) {
        let key = (counter, kline_type);

        let mut store = self.inner.write().expect("poison");
        let entry = store.entry(key).or_insert((true, vec![]));
        entry.0 = more;

        for kline in data {
            if let Some(existing) = entry.1.iter_mut().find(|k| k.timestamp == kline.timestamp) {
                *existing = kline;
            } else {
                entry.1.push(kline);
            }
        }

        entry.1.sort_by_key(|k| k.timestamp);
    }
    pub fn apply_trade(&self, counter: &Counter, timestamp: i64, price: Decimal, volume_raw: i64) {
        if volume_raw == 0 {
            return;
        }

        let mut store = self.inner.write().expect("poison");
        for ((entry_counter, kline_type), (_has_more, klines)) in store.iter_mut() {
            if entry_counter != counter {
                continue;
            }

            let bucket_ts = bucket_start_timestamp(timestamp, *kline_type);
            if let Some(existing) = klines.iter_mut().find(|k| k.timestamp == bucket_ts) {
                apply_trade_to_kline(existing, counter, price, volume_raw);
                continue;
            }

            if klines.last().is_none_or(|last| last.timestamp < bucket_ts) {
                let mut next = Kline {
                    timestamp: bucket_ts,
                    open: price,
                    high: price,
                    low: price,
                    close: price,
                    amount: 0,
                    balance: Decimal::ZERO,
                    total: 0,
                };
                apply_trade_to_kline(&mut next, counter, price, volume_raw);
                klines.push(next);
            }
        }
    }

    async fn request(counter: Counter, kline_type: KlineType, _before: i64, count: usize) {
        let ctx = crate::openapi::quote();

        let timeframe = match kline_type {
            KlineType::PerMinute => "1m",
            KlineType::PerFiveMinutes => "5m",
            KlineType::PerFifteenMinutes => "15m",
            KlineType::PerThirtyMinutes => "1h",
            KlineType::PerHour => "1h",
            KlineType::PerDay => "1d",
            KlineType::PerWeek | KlineType::PerMonth | KlineType::PerYear => "1d",
        };

        tracing::info!(
            "Requesting candlestick data: counter={}, timeframe={}, count={}",
            counter,
            timeframe,
            count
        );

        match ctx.ohlcv(counter.as_str(), timeframe, count.min(100)).await {
            Ok(candles) => {
                tracing::info!(
                    "Successfully fetched candlestick data: counter={}, count={}",
                    counter,
                    candles.len()
                );

                let klines: Vec<Kline> = candles
                    .iter()
                    .map(|c| Kline {
                        timestamp: c.timestamp.unix_timestamp(),
                        open: c.open,
                        high: c.high,
                        low: c.low,
                        close: c.close,
                        amount: c
                            .volume_base
                            .round_dp(0)
                            .to_string()
                            .parse::<u64>()
                            .unwrap_or_default(),
                        balance: c.volume_quote,
                        total: c.trade_count,
                    })
                    .collect();

                if !klines.is_empty() {
                    tracing::debug!(
                        "First candlestick: open={}, high={}, low={}, close={}, volume={}",
                        klines[0].open,
                        klines[0].high,
                        klines[0].low,
                        klines[0].close,
                        klines[0].amount
                    );
                }

                let has_more = klines.len() == count;
                KLINES.update(counter, kline_type, klines, has_more);
            }
            Err(e) => {
                tracing::error!(
                    "Failed to request candlestick data: counter={}, error={}",
                    counter,
                    e
                );
            }
        }
    }
}

fn apply_trade_to_kline(kline: &mut Kline, counter: &Counter, price: Decimal, volume_raw: i64) {
    if kline.open == Decimal::ZERO {
        kline.open = price;
    }
    if kline.high == Decimal::ZERO || price > kline.high {
        kline.high = price;
    }
    if kline.low == Decimal::ZERO || price < kline.low {
        kline.low = price;
    }
    kline.close = price;

    let volume_u64 = volume_raw.unsigned_abs();
    kline.amount = kline.amount.saturating_add(volume_u64);

    let normalized_volume =
        crate::openapi::quote::normalize_base_volume_i64(counter.as_str(), volume_raw).abs();
    let quote_volume = (price * normalized_volume).abs();
    kline.balance += quote_volume;
    kline.total = kline.total.saturating_add(1);
}

fn bucket_start_timestamp(timestamp: i64, kline_type: KlineType) -> i64 {
    let interval_secs: i64 = match kline_type {
        KlineType::PerMinute => 60,
        KlineType::PerFiveMinutes => 300,
        KlineType::PerFifteenMinutes => 900,
        KlineType::PerThirtyMinutes | KlineType::PerHour => 3_600,
        KlineType::PerDay | KlineType::PerWeek | KlineType::PerMonth | KlineType::PerYear => 86_400,
    };

    timestamp - timestamp.rem_euclid(interval_secs)
}
