use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Counter {
    inner: String,
}

impl Counter {
    pub fn new(symbol: &str) -> Self {
        Self {
            inner: symbol.to_string(),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.inner
    }

    pub fn code(&self) -> &str {
        if self.as_str().contains('_') {
            self.as_str().split('_').next().unwrap_or("")
        } else {
            self.as_str().split('.').next().unwrap_or("")
        }
    }

    pub fn market(&self) -> &str {
        if self.as_str().contains('_') {
            self.as_str().split('_').nth(1).unwrap_or("")
        } else {
            self.as_str().split('.').nth(1).unwrap_or("")
        }
    }
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

impl std::fmt::Display for Counter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl From<&str> for Counter {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl std::str::FromStr for Counter {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::new(s))
    }
}

impl From<String> for Counter {
    fn from(s: String) -> Self {
        Self::new(&s)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PriceColorMode {
    #[default]
    RedUp,
    GreenUp,
}
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    bytemuck::NoUninit,
    strum::EnumIter,
)]
#[repr(u8)]
#[derive(Default)]
pub enum KlineType {
    PerMinute = 0,
    PerFiveMinutes = 1,
    PerFifteenMinutes = 2,
    PerThirtyMinutes = 3,
    PerHour = 4,
    #[default]
    PerDay = 5,
    PerWeek = 6,
    PerMonth = 7,
    PerYear = 8,
}

impl std::fmt::Display for KlineType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PerMinute => write!(f, "1m"),
            Self::PerFiveMinutes => write!(f, "5m"),
            Self::PerFifteenMinutes => write!(f, "15m"),
            Self::PerThirtyMinutes => write!(f, "30m"),
            Self::PerHour => write!(f, "1h"),
            Self::PerDay => write!(f, "Day"),
            Self::PerWeek => write!(f, "Week"),
            Self::PerMonth => write!(f, "Month"),
            Self::PerYear => write!(f, "Year"),
        }
    }
}

impl KlineType {
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::PerMinute => Self::PerFiveMinutes,
            Self::PerFiveMinutes => Self::PerFifteenMinutes,
            Self::PerFifteenMinutes => Self::PerThirtyMinutes,
            Self::PerThirtyMinutes => Self::PerHour,
            Self::PerHour => Self::PerDay,
            Self::PerDay => Self::PerWeek,
            Self::PerWeek => Self::PerMonth,
            Self::PerMonth | Self::PerYear => Self::PerYear,
        }
    }
    #[must_use]
    pub fn prev(self) -> Self {
        match self {
            Self::PerMinute | Self::PerFiveMinutes => Self::PerMinute,
            Self::PerFifteenMinutes => Self::PerFiveMinutes,
            Self::PerThirtyMinutes => Self::PerFifteenMinutes,
            Self::PerHour => Self::PerThirtyMinutes,
            Self::PerDay => Self::PerHour,
            Self::PerWeek => Self::PerDay,
            Self::PerMonth => Self::PerWeek,
            Self::PerYear => Self::PerMonth,
        }
    }

    pub fn iter() -> impl Iterator<Item = Self> {
        <Self as strum::IntoEnumIterator>::iter()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Kline {
    pub timestamp: i64,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
    pub amount: u64,
    pub balance: Decimal,
    pub total: u64,
}

impl Default for Kline {
    fn default() -> Self {
        Self {
            timestamp: 0,
            open: Decimal::ZERO,
            high: Decimal::ZERO,
            low: Decimal::ZERO,
            close: Decimal::ZERO,
            amount: 0,
            balance: Decimal::ZERO,
            total: 0,
        }
    }
}
pub type Klines = Vec<Kline>;
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
pub enum Market {
    #[default]
    Crypto,
}

impl From<&str> for Market {
    fn from(_s: &str) -> Self {
        Self::Crypto
    }
}

impl Market {
    pub fn as_str(&self) -> &'static str {
        "CRYPTO"
    }
}

impl std::fmt::Display for Market {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QuoteData {
    pub last_done: Option<Decimal>,
    pub prev_close: Option<Decimal>,
    pub open: Option<Decimal>,
    pub high: Option<Decimal>,
    pub low: Option<Decimal>,
    pub volume: u64,
    pub quote_volume: Decimal,
    pub timestamp: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candlestick {
    pub timestamp: i64,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
    pub volume: u64,
    pub quote_volume: Decimal,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Depth {
    pub position: i32,
    pub price: Decimal,
    pub volume: i64,
    pub order_num: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DepthData {
    pub asks: Vec<Depth>,
    pub bids: Vec<Depth>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeDirection {
    Neutral,
    Up,
    Down,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TradeData {
    pub price: Decimal,
    pub volume: i64,
    pub timestamp: i64,
    pub trade_type: String,
    pub direction: TradeDirection,
}

impl Default for TradeData {
    fn default() -> Self {
        Self {
            price: Decimal::ZERO,
            volume: 0,
            timestamp: 0,
            trade_type: String::new(),
            direction: TradeDirection::Neutral,
        }
    }
}
