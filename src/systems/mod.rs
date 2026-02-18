use std::{collections::HashMap, sync::Mutex};

use atomic::Atomic;
use bevy_ecs::{
    event::Event,
    schedule::State,
    system::{Res, ResMut, Resource},
};
use ratatui::widgets::TableState;
use rust_decimal::Decimal;
use tokio::sync::mpsc;

use crate::{
    app::AppState,
    data::{Counter, KlineType, ReadyState, WatchlistGroup},
    openapi,
    widgets::{Carousel, LocalSearch, Search},
};

mod common;
mod pool_detail;
mod watchlist;
mod watchlist_pool;

// Re-export render functions
pub use common::*;
pub use pool_detail::*;
pub use watchlist::*;
pub use watchlist_pool::*;
pub struct WsManager;

impl WsManager {
    pub async fn remount(&self, _name: &str, symbols: &[Counter]) -> anyhow::Result<()> {
        let ctx = crate::openapi::quote();
        let symbol_strings: Vec<String> = symbols.iter().map(ToString::to_string).collect();
        let _ = ctx
            .subscribe(&symbol_strings, openapi::SubFlags::QUOTE)
            .await;
        Ok(())
    }

    pub async fn quote_detail(&self, _name: &str, symbols: &[Counter]) -> anyhow::Result<()> {
        let ctx = crate::openapi::quote();
        let symbol_strings: Vec<String> = symbols.iter().map(ToString::to_string).collect();
        let _ = ctx
            .subscribe(
                &symbol_strings,
                openapi::SubFlags::QUOTE | openapi::SubFlags::DEPTH,
            )
            .await;
        Ok(())
    }

    pub async fn quote_trade(&self, _name: &str, symbols: &[Counter]) -> anyhow::Result<()> {
        let ctx = crate::openapi::quote();
        let symbol_strings: Vec<String> = symbols.iter().map(ToString::to_string).collect();
        let _ = ctx
            .subscribe(&symbol_strings, openapi::SubFlags::TRADE)
            .await;
        Ok(())
    }
}

pub static WS: std::sync::LazyLock<WsManager> = std::sync::LazyLock::new(|| WsManager);

#[derive(Event)]
pub struct TuiEvent(pub tui_input::InputRequest);
pub(crate) static KLINE_TYPE: Atomic<KlineType> = Atomic::new(KlineType::PerDay);
pub(crate) static KLINE_INDEX: Atomic<usize> = Atomic::new(0);

pub(crate) static LAST_DONE: std::sync::LazyLock<Mutex<HashMap<Counter, Decimal>>> =
    std::sync::LazyLock::new(Mutex::default);
pub(crate) static WATCHLIST_TABLE: std::sync::LazyLock<Mutex<TableState>> =
    std::sync::LazyLock::new(Mutex::default);
pub(crate) type NavFooter<'w> = (
    Res<'w, State<AppState>>,
    Res<'w, Carousel<[Counter; 3]>>,
    Res<'w, WsState>,
);

pub(crate) type PopUp<'w> = (
    ResMut<'w, Search<openapi::search::PoolItem>>,
    ResMut<'w, LocalSearch<WatchlistGroup>>,
);
#[derive(Event)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Tab,
    BackTab,
    Enter,
}
#[derive(Clone, Resource)]
pub struct Command(pub mpsc::UnboundedSender<bevy_ecs::system::CommandQueue>);

#[derive(Resource)]
pub struct WsState(pub ReadyState);

#[derive(Resource)]
pub struct PoolDetail(pub Counter);
