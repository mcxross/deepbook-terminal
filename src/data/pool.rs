use serde::{Deserialize, Serialize};

use super::types::{Counter, Depth, DepthData, QuoteData, TradeData};
use crate::openapi::types::{
    PushQuote, SecurityDepth, SecurityQuote, Trade, TradeDirection as ApiTradeDirection,
    TradeStatus,
};

type QuoteSnapshot = SecurityQuote;
type OrderBookSnapshot = SecurityDepth;

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Pool {
    pub counter: Counter,
    pub name: String,
    pub status: String,
    pub quote: QuoteData,
    pub depth: DepthData,
    pub trades: Vec<TradeData>,
}

impl Pool {
    pub fn new(counter: Counter) -> Self {
        Self {
            counter,
            name: String::new(),
            status: "Trading".to_string(),
            quote: QuoteData::default(),
            depth: DepthData::default(),
            trades: Vec::new(),
        }
    }

    pub fn quoting(&self) -> bool {
        true
    }

    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            self.counter.code()
        } else {
            &self.name
        }
    }

    pub fn update_from_push_quote(&mut self, quote: &PushQuote) {
        self.quote.last_done = Some(quote.last_done);
        self.quote.open = Some(quote.open);
        self.quote.high = Some(quote.high);
        self.quote.low = Some(quote.low);
        self.quote.volume = quote.volume.cast_unsigned();
        self.quote.quote_volume = quote.turnover;
        self.quote.timestamp = quote.timestamp.unix_timestamp();
        self.status = map_pool_status(quote.trade_status);
    }

    pub fn update_from_quote(&mut self, quote: &QuoteSnapshot) {
        self.quote.last_done = Some(quote.last_done);
        self.quote.prev_close = Some(quote.prev_close);
        self.quote.open = Some(quote.open);
        self.quote.high = Some(quote.high);
        self.quote.low = Some(quote.low);
        self.quote.volume = quote.volume.cast_unsigned();
        self.quote.quote_volume = quote.turnover;
        self.quote.timestamp = quote.timestamp.unix_timestamp();
        self.status = map_pool_status(quote.trade_status);
    }

    pub fn update_from_depth(&mut self, depth: &OrderBookSnapshot) {
        self.depth.asks = depth
            .asks
            .iter()
            .map(|d| Depth {
                position: d.position,
                price: d.price.unwrap_or_default(),
                volume: d.volume,
                order_num: d.order_num,
            })
            .collect();

        self.depth.bids = depth
            .bids
            .iter()
            .map(|d| Depth {
                position: d.position,
                price: d.price.unwrap_or_default(),
                volume: d.volume,
                order_num: d.order_num,
            })
            .collect();
    }

    pub fn update_from_trades(&mut self, trades: &[Trade]) {
        self.trades = trades
            .iter()
            .map(|t| TradeData {
                price: t.price,
                volume: t.volume,
                timestamp: t.timestamp.unix_timestamp(),
                trade_type: t.trade_type.clone(),
                direction: match t.direction {
                    ApiTradeDirection::Neutral => super::types::TradeDirection::Neutral,
                    ApiTradeDirection::Down => super::types::TradeDirection::Down,
                    ApiTradeDirection::Up => super::types::TradeDirection::Up,
                },
            })
            .collect();
    }
}

fn map_pool_status(status: TradeStatus) -> String {
    if status == TradeStatus::Normal {
        "Trading".to_string()
    } else {
        "Unavailable".to_string()
    }
}
