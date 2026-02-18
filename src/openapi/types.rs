use rust_decimal::Decimal;
use time::OffsetDateTime;

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct SubFlags: u8 {
        const QUOTE = 0b0000_0001;
        const DEPTH = 0b0000_0010;
        const TRADE = 0b0000_0100;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TradeStatus {
    Normal,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TradeSession {
    Intraday,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TradeDirection {
    Neutral,
    Up,
    Down,
}

#[derive(Clone, Debug)]
pub struct PushQuote {
    pub last_done: Decimal,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub timestamp: OffsetDateTime,
    pub volume: i64,
    pub turnover: Decimal,
    pub trade_status: TradeStatus,
    pub trade_session: TradeSession,
    pub current_volume: i64,
    pub current_turnover: Decimal,
}

#[derive(Clone, Debug)]
pub struct Trade {
    pub price: Decimal,
    pub volume: i64,
    pub timestamp: OffsetDateTime,
    pub trade_type: String,
    pub direction: TradeDirection,
    pub trade_session: TradeSession,
}

#[derive(Clone, Debug)]
pub struct Depth {
    pub position: i32,
    pub price: Option<Decimal>,
    pub volume: i64,
    pub order_num: i64,
}

#[derive(Clone, Debug, Default)]
pub struct SecurityDepth {
    pub asks: Vec<Depth>,
    pub bids: Vec<Depth>,
}

#[derive(Clone, Debug)]
pub struct SecurityQuote {
    pub symbol: String,
    pub last_done: Decimal,
    pub prev_close: Decimal,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub timestamp: OffsetDateTime,
    pub volume: i64,
    pub turnover: Decimal,
    pub trade_status: TradeStatus,
    pub pre_market_quote: Option<()>,
    pub post_market_quote: Option<()>,
    pub overnight_quote: Option<()>,
}
