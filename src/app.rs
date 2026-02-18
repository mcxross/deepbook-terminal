use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};

use atomic::Atomic;
use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use bevy_ecs::system::{CommandQueue, InsertResource, SystemState};
use tokio::sync::mpsc;

use crate::data::{Counter, Watchlist, WatchlistGroup};
use crate::render::{DirtyFlags, RenderState};
use crate::ui::Content;
use crate::widgets::{Carousel, Loading, LocalSearch, Search, Terminal};
use crate::{openapi, systems};

pub static RT: OnceLock<tokio::runtime::Handle> = OnceLock::new();
pub static POPUP: AtomicU8 = AtomicU8::new(0);
pub static LAST_STATE: Atomic<AppState> = Atomic::new(AppState::Watchlist);
pub static QUOTE_BMP: Atomic<bool> = Atomic::new(false);
pub static LOG_PANEL_VISIBLE: Atomic<bool> = Atomic::new(false);
pub static WATCHLIST: std::sync::LazyLock<RwLock<Watchlist>> =
    std::sync::LazyLock::new(Default::default);

pub const POPUP_HELP: u8 = 0b1;
pub const POPUP_SEARCH: u8 = 0b10;
pub const POPUP_WATCHLIST: u8 = 0b100;

#[derive(
    Clone, Copy, PartialEq, Eq, Hash, Debug, Default, States, strum::EnumIter, bytemuck::NoUninit,
)]
#[repr(u8)]
pub enum AppState {
    Error,
    #[default]
    Loading,
    Pool,
    Watchlist,
    WatchlistPool,
}

#[allow(clippy::too_many_lines)]
pub async fn run(
    _args: crate::Args,
    mut quote_receiver: impl tokio_stream::Stream<Item = openapi::PushEvent> + Unpin,
) {
    let (update_tx, mut update_rx) = mpsc::unbounded_channel();

    let mut footer_symbols: Vec<Counter> = openapi::quote()
        .configured_symbols()
        .into_iter()
        .take(3)
        .map(Counter::from)
        .collect();
    while footer_symbols.len() < 3 {
        footer_symbols.push(Counter::from(""));
    }

    let indexes: Vec<[Counter; 3]> = vec![[
        footer_symbols[0].clone(),
        footer_symbols[1].clone(),
        footer_symbols[2].clone(),
    ]];

    let subs: Vec<Counter> = indexes.iter().flatten().cloned().collect();
    tokio::spawn({
        let subs = subs.clone();
        async move {
            let ctx = crate::openapi::quote();
            let symbols: Vec<String> = subs
                .iter()
                .map(std::string::ToString::to_string)
                .filter(|s| !s.is_empty())
                .collect();

            if symbols.is_empty() {
                return;
            }

            match ctx.quote(&symbols).await {
                Ok(quotes) => {
                    tracing::info!("Fetched {} index quotes", quotes.len());
                    for quote in quotes {
                        let counter = Counter::new(&quote.symbol);
                        let mut pool = crate::data::Pool::new(counter);
                        pool.update_from_quote(&quote);
                        crate::data::POOLS.insert(pool);
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to fetch index quotes: {}", e);
                }
            }

            if let Err(e) = ctx.subscribe(&symbols, openapi::SubFlags::QUOTE).await {
                tracing::error!("Failed to subscribe indexes: {}", e);
            } else {
                tracing::info!("Successfully subscribed to {} indexes", symbols.len());
            }
        }
    });

    let search_pool = Search::new(update_tx.clone(), |keyword| {
        Box::pin(async move {
            let query = openapi::search::PoolQuery { keyword };
            openapi::search::fetch_pool(&query)
                .await
                .map(|v| v.product_list)
                .unwrap_or_default()
        })
    });
    let search_watchlist = LocalSearch::new(Vec::<WatchlistGroup>::new(), |_keyword, _group| false);

    RT.set(tokio::runtime::Handle::current()).unwrap();
    let mut app = bevy_app::App::new();
    app.add_state::<AppState>()
        .add_event::<systems::Key>()
        .add_event::<systems::TuiEvent>()
        .init_resource::<Terminal>()
        .init_resource::<Loading>()
        .insert_resource(search_pool)
        .insert_resource(search_watchlist)
        .insert_resource(systems::Command(update_tx.clone()))
        .insert_resource(Carousel::new(indexes, Duration::from_secs(5)))
        .insert_resource(systems::WsState(crate::data::ReadyState::Open))
        .add_systems(Update, systems::loading.run_if(in_state(AppState::Loading)))
        .add_systems(Update, systems::error.run_if(in_state(AppState::Error)))
        .add_systems(OnExit(AppState::Watchlist), systems::exit_watchlist)
        .add_systems(
            Update,
            systems::render_watchlist.run_if(in_state(AppState::Watchlist)),
        )
        .add_systems(OnEnter(AppState::Pool), systems::enter_pool)
        .add_systems(OnExit(AppState::Pool), systems::exit_pool)
        .add_systems(
            Update,
            systems::render_pool.run_if(in_state(AppState::Pool)),
        )
        .add_systems(OnEnter(AppState::WatchlistPool), systems::enter_pool)
        .add_systems(OnExit(AppState::WatchlistPool), systems::exit_pool)
        .add_systems(
            Update,
            systems::render_watchlist_pool.run_if(in_state(AppState::WatchlistPool)),
        );

    for v in <AppState as strum::IntoEnumIterator>::iter() {
        if v == AppState::Watchlist || v == AppState::WatchlistPool {
            continue;
        }
        for watch in [AppState::Watchlist, AppState::WatchlistPool] {
            app.add_systems(
                OnTransition { from: v, to: watch },
                systems::enter_watchlist_common,
            );
            app.add_systems(
                OnTransition { from: watch, to: v },
                systems::exit_watchlist_common,
            );
        }
    }

    _ = update_tx.send({
        let mut queue = CommandQueue::default();
        queue.push(InsertResource {
            resource: NextState(Some(AppState::Watchlist)),
        });
        queue
    });
    systems::refresh_watchlist(update_tx.clone());

    tokio::spawn({
        let tx = update_tx.clone();
        async move {
            use std::fs;
            use std::path::PathBuf;
            use std::time::SystemTime;

            let mut last_modified: Option<SystemTime> = None;
            let mut last_size: u64 = 0;

            let get_latest_log_file = || -> Option<PathBuf> {
                let log_dir = crate::logger::default_log_dir();
                let mut log_files: Vec<PathBuf> = fs::read_dir(&log_dir)
                    .ok()?
                    .filter_map(std::result::Result::ok)
                    .map(|entry| entry.path())
                    .filter(|path| {
                        path.is_file()
                            && path.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                                n.starts_with("surflux-terminal")
                                    && std::path::Path::new(n)
                                        .extension()
                                        .is_some_and(|ext| ext.eq_ignore_ascii_case("log"))
                            })
                    })
                    .collect();

                log_files.sort_by(|a, b| {
                    let time_a = fs::metadata(a).and_then(|m| m.modified()).ok();
                    let time_b = fs::metadata(b).and_then(|m| m.modified()).ok();
                    match (time_a, time_b) {
                        (Some(ta), Some(tb)) => tb.cmp(&ta),
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => std::cmp::Ordering::Equal,
                    }
                });

                log_files.into_iter().next()
            };

            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;

                if !LOG_PANEL_VISIBLE.load(Ordering::Relaxed) {
                    continue;
                }

                if let Some(log_file) = get_latest_log_file() {
                    if let Ok(metadata) = fs::metadata(&log_file) {
                        let modified = metadata.modified().ok();
                        let size = metadata.len();

                        if modified != last_modified || size != last_size {
                            last_modified = modified;
                            last_size = size;

                            let queue = CommandQueue::default();
                            _ = tx.send(queue);
                        }
                    }
                }
            }
        }
    });

    let mut render_tick = tokio::time::interval(std::time::Duration::from_millis(16));
    render_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_render_at = Instant::now();

    let mut pending_depth: HashMap<Counter, openapi::StreamDepth> = HashMap::new();
    let mut pending_trades: HashMap<Counter, Vec<crate::data::TradeData>> = HashMap::new();

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut events = crossterm::event::EventStream::new();
    let mut render_state = RenderState::new();

    render_state.mark_all_dirty();

    loop {
        tokio::select! {
            _ = render_tick.tick() => {
                if !pending_depth.is_empty() {
                    for (counter, depth) in pending_depth.drain() {
                        crate::data::POOLS.modify(counter, |pool| {
                            use rust_decimal::Decimal;
                            pool.depth.asks = depth
                                .asks
                                .iter()
                                .map(|d| crate::data::Depth {
                                    position: d.position,
                                    price: d.price.unwrap_or(Decimal::ZERO),
                                    volume: d.volume,
                                    order_num: d.order_num,
                                })
                                .collect();
                            pool.depth.bids = depth
                                .bids
                                .iter()
                                .map(|d| crate::data::Depth {
                                    position: d.position,
                                    price: d.price.unwrap_or(Decimal::ZERO),
                                    volume: d.volume,
                                    order_num: d.order_num,
                                })
                                .collect();
                        });
                    }
                    render_state.mark_dirty(DirtyFlags::NONE.mark_depth_update());
                }

                if !pending_trades.is_empty() {
                    for (counter, trades_batch) in pending_trades.drain() {
                        let mut trades_for_kline = trades_batch.clone();
                        trades_for_kline.sort_by_key(|t| t.timestamp);
                        for trade in &trades_for_kline {
                            crate::kline::KLINES.apply_trade(
                                &counter,
                                trade.timestamp,
                                trade.price,
                                trade.volume,
                            );
                        }

                        crate::data::POOLS.modify(counter, |pool| {
                            pool.trades.splice(0..0, trades_batch);
                            pool.trades.truncate(50);
                        });
                    }
                    render_state.mark_dirty(DirtyFlags::POOL_DETAIL);
                }

                let state = *app.world.resource::<State<AppState>>().get();
                let min_interval = match state {
                    AppState::Pool | AppState::WatchlistPool => Duration::from_millis(16),
                    _ => Duration::from_millis(33),
                };
                if last_render_at.elapsed() < min_interval {
                    continue;
                }

                if render_state.needs_render() {
                    app.update();
                    render_state.clear();
                    last_render_at = Instant::now();
                } else {
                    render_state.skip();
                }
            }

            Some(mut cmd) = update_rx.recv() => {
                cmd.apply(&mut app.world);
                render_state.mark_dirty(DirtyFlags::ALL);
            }

            Some(push_event) = tokio_stream::StreamExt::next(&mut quote_receiver) => {
                let symbol = push_event.symbol;
                let counter = Counter::new(&symbol);
                 match push_event.detail {
                     openapi::PushEventDetail::Quote(quote) => {
                         tracing::debug!(
                             "Update quote: {} = {}",
                             symbol,
                             quote.last_done
                         );
                         crate::data::POOLS.modify(counter.clone(), |pool| {
                             pool.update_from_push_quote(&quote);
                         });
                         render_state.mark_dirty(DirtyFlags::NONE.mark_quote_update());
                     }
                     openapi::PushEventDetail::Depth(depth) => {
                         tracing::debug!("Update depth: {}", symbol);
                         pending_depth.insert(counter, depth);
                     }
                     openapi::PushEventDetail::Trades(trades) => {
                         if !trades.is_empty() {
                             let mapped = trades
                                 .into_iter()
                                 .map(|trade| crate::data::TradeData {
                                     price: trade.price,
                                     volume: trade.volume,
                                     timestamp: trade.timestamp.unix_timestamp(),
                                     trade_type: trade.trade_type,
                                     direction: match trade.direction {
                                         openapi::TradeDirection::Up => crate::data::TradeDirection::Up,
                                         openapi::TradeDirection::Down => crate::data::TradeDirection::Down,
                                         openapi::TradeDirection::Neutral => crate::data::TradeDirection::Neutral,
                                     },
                                 })
                                 .collect::<Vec<_>>();
                             let entry = pending_trades.entry(counter).or_default();
                             entry.splice(0..0, mapped);
                             entry.truncate(200);
                         }
                     }
                 }
            }

            Some(event) = tokio_stream::StreamExt::next(&mut events) => {
                let event = match event {
                    Ok(crossterm::event::Event::Key(event)) => event,
                    Ok(_) => {
                        continue
                    },
                    Err(err) => {
                        tracing::error!("fail to receive event: {err}");
                        app.world.insert_resource(Content::new(
                            "Failed to create QR code",
                            "Please quit the app and try again",
                        ));
                        app.world.insert_resource(NextState(Some(AppState::Error)));
                        render_state.mark_dirty(DirtyFlags::ERROR);
                        continue;
                    }
                };

                let popup = POPUP.load(Ordering::Relaxed);
                let state = *app.world.resource::<State<AppState>>().get();

                if event.code == crossterm::event::KeyCode::Char('`')
                    && event.modifiers == crossterm::event::KeyModifiers::NONE {
                    let was_visible = LOG_PANEL_VISIBLE.load(Ordering::Relaxed);
                    LOG_PANEL_VISIBLE.store(!was_visible, Ordering::Relaxed);
                    render_state.mark_dirty(DirtyFlags::ALL);
                    continue;
                }

                if popup != 0 {
                    handle_popup_input(&mut app, popup, event, update_tx.clone());
                    render_state.mark_dirty(DirtyFlags::NONE.mark_popup_change(popup));
                    continue;
                }

                match state {
                    AppState::Error => return,
                    AppState::Loading => {
                        if matches!(event, ctrl!('c') | key!('q')) {
                            return;
                        }
                        continue;
                    }
                    AppState::Pool | AppState::Watchlist | AppState::WatchlistPool => (),
                }

                handle_global_keys(&mut app, event, state, update_tx.clone(), &mut render_state);
            }
        }
    }
}

fn handle_popup_input(
    app: &mut App,
    popup: u8,
    event: crossterm::event::KeyEvent,
    update_tx: mpsc::UnboundedSender<CommandQueue>,
) {
    if popup == POPUP_WATCHLIST {
        let mut search = app.world.resource_mut::<LocalSearch<WatchlistGroup>>();
        let (hidden, selected) = search.handle_key(event);
        if hidden {
            POPUP.store(0, Ordering::Relaxed);
        }
        if let Some(group) = selected {
            POPUP.store(0, Ordering::Relaxed);
            WATCHLIST.write().expect("poison").set_group_id(group.id);
            systems::refresh_watchlist(update_tx.clone());
        }
    } else if popup == POPUP_SEARCH {
        let mut search = app
            .world
            .resource_mut::<Search<openapi::search::PoolItem>>();
        let (hidden, selected) = search.handle_key(event);
        if hidden {
            POPUP.store(0, Ordering::Relaxed);
        }
        if let Some(selected) = selected {
            POPUP.store(0, Ordering::Relaxed);
            app.world
                .insert_resource(systems::PoolDetail(selected.counter_id));
            let state = *app.world.resource::<State<AppState>>().get();
            let next_state = if state == AppState::Pool {
                AppState::Pool
            } else {
                AppState::WatchlistPool
            };
            app.world.insert_resource(NextState(Some(next_state)));
        }
    } else if popup == POPUP_HELP {
        POPUP.store(0, Ordering::Relaxed);
    }
}

#[allow(clippy::too_many_lines)]
fn handle_global_keys(
    app: &mut App,
    event: crossterm::event::KeyEvent,
    state: AppState,
    update_tx: mpsc::UnboundedSender<CommandQueue>,
    render_state: &mut RenderState,
) {
    match event {
        ctrl!('c') => Terminal::graceful_exit(0),
        key!('1') if state != AppState::Watchlist => {
            app.world
                .insert_resource(NextState(Some(AppState::Watchlist)));
            render_state.mark_dirty(DirtyFlags::ALL);
        }
        ::crossterm::event::KeyEvent {
            code: ::crossterm::event::KeyCode::Char('g' | 'G'),
            modifiers: ::crossterm::event::KeyModifiers::NONE,
            kind: ::crossterm::event::KeyEventKind::Press,
            state: ::crossterm::event::KeyEventState::NONE,
        } if state == AppState::Watchlist || state == AppState::WatchlistPool => {
            if let Some(mut search) = app.world.get_resource_mut::<LocalSearch<WatchlistGroup>>() {
                POPUP.store(POPUP_WATCHLIST, Ordering::Relaxed);
                search.visible();
                render_state.mark_dirty(DirtyFlags::POPUP_WATCHLIST);
            }
        }
        ::crossterm::event::KeyEvent {
            code: ::crossterm::event::KeyCode::Char('Q'),
            modifiers:
                ::crossterm::event::KeyModifiers::NONE | ::crossterm::event::KeyModifiers::SHIFT,
            kind: ::crossterm::event::KeyEventKind::Press,
            state: ::crossterm::event::KeyEventState::NONE,
        } => {
            show_index(&mut app.world, 0);
            render_state.mark_dirty(DirtyFlags::POOL_DETAIL | DirtyFlags::WATCHLIST);
        }
        ::crossterm::event::KeyEvent {
            code: ::crossterm::event::KeyCode::Char('W'),
            modifiers:
                ::crossterm::event::KeyModifiers::NONE | ::crossterm::event::KeyModifiers::SHIFT,
            kind: ::crossterm::event::KeyEventKind::Press,
            state: ::crossterm::event::KeyEventState::NONE,
        } => {
            show_index(&mut app.world, 1);
            render_state.mark_dirty(DirtyFlags::POOL_DETAIL | DirtyFlags::WATCHLIST);
        }
        ::crossterm::event::KeyEvent {
            code: ::crossterm::event::KeyCode::Char('E'),
            modifiers:
                ::crossterm::event::KeyModifiers::NONE | ::crossterm::event::KeyModifiers::SHIFT,
            kind: ::crossterm::event::KeyEventKind::Press,
            state: ::crossterm::event::KeyEventState::NONE,
        } => {
            show_index(&mut app.world, 2);
            render_state.mark_dirty(DirtyFlags::POOL_DETAIL | DirtyFlags::WATCHLIST);
        }
        ::crossterm::event::KeyEvent {
            code: ::crossterm::event::KeyCode::Char('t'),
            modifiers:
                ::crossterm::event::KeyModifiers::NONE | ::crossterm::event::KeyModifiers::SHIFT,
            kind: ::crossterm::event::KeyEventKind::Press,
            state: ::crossterm::event::KeyEventState::NONE,
        } => {
            if state == AppState::Pool {
                app.world
                    .insert_resource(NextState(Some(AppState::WatchlistPool)));
                render_state.mark_dirty(DirtyFlags::POOL_DETAIL | DirtyFlags::WATCHLIST);
            } else if state == AppState::WatchlistPool {
                app.world.insert_resource(NextState(Some(AppState::Pool)));
                render_state.mark_dirty(DirtyFlags::POOL_DETAIL | DirtyFlags::WATCHLIST);
            }
        }
        ::crossterm::event::KeyEvent {
            code: ::crossterm::event::KeyCode::Char('R'),
            modifiers:
                ::crossterm::event::KeyModifiers::NONE | ::crossterm::event::KeyModifiers::SHIFT,
            kind: ::crossterm::event::KeyEventKind::Press,
            state: ::crossterm::event::KeyEventState::NONE,
        } => match state {
            AppState::Watchlist => {
                systems::refresh_watchlist(update_tx.clone());
                render_state.mark_dirty(DirtyFlags::WATCHLIST);
            }
            AppState::WatchlistPool => {
                systems::refresh_pool_debounced(
                    app.world.resource::<systems::PoolDetail>().0.clone(),
                );
                systems::refresh_watchlist(update_tx.clone());
                render_state.mark_dirty(DirtyFlags::POOL_DETAIL | DirtyFlags::WATCHLIST);
            }
            AppState::Pool => {
                systems::refresh_pool_debounced(
                    app.world.resource::<systems::PoolDetail>().0.clone(),
                );
                render_state.mark_dirty(DirtyFlags::POOL_DETAIL);
            }
            _ => {}
        },
        key!('?') => {
            POPUP.store(POPUP_HELP, Ordering::Relaxed);
            render_state.mark_dirty(DirtyFlags::POPUP_HELP);
        }
        key!('/') => {
            if let Some(mut search) = app
                .world
                .get_resource_mut::<Search<openapi::search::PoolItem>>()
            {
                POPUP.store(POPUP_SEARCH, Ordering::Relaxed);
                search.visible();
                render_state.mark_dirty(DirtyFlags::POPUP_SEARCH);
            }
        }
        ::crossterm::event::KeyEvent {
            code: ::crossterm::event::KeyCode::Esc | ::crossterm::event::KeyCode::Char('q'),
            modifiers: ::crossterm::event::KeyModifiers::NONE,
            kind: ::crossterm::event::KeyEventKind::Press,
            state: ::crossterm::event::KeyEventState::NONE,
        } => {
            let last_state = LAST_STATE.load(Ordering::Relaxed);
            if last_state != state {
                app.world.insert_resource(NextState(Some(last_state)));
                render_state.mark_dirty(DirtyFlags::ALL);
            }
        }
        ::crossterm::event::KeyEvent {
            code: ::crossterm::event::KeyCode::Up | ::crossterm::event::KeyCode::Char('k'),
            modifiers: ::crossterm::event::KeyModifiers::NONE,
            kind: ::crossterm::event::KeyEventKind::Press,
            state: ::crossterm::event::KeyEventState::NONE,
        }
        | ::crossterm::event::KeyEvent {
            code: ::crossterm::event::KeyCode::Char('k'),
            modifiers: ::crossterm::event::KeyModifiers::SHIFT,
            kind: ::crossterm::event::KeyEventKind::Press,
            state: ::crossterm::event::KeyEventState::NONE,
        } => {
            send_evt(systems::Key::Up, &mut app.world);
            // Navigation keys affect current view
            render_state.mark_dirty(match state {
                AppState::Watchlist | AppState::WatchlistPool => DirtyFlags::WATCHLIST,
                AppState::Pool => DirtyFlags::POOL_DETAIL,
                _ => DirtyFlags::ALL,
            });
        }
        ::crossterm::event::KeyEvent {
            code: ::crossterm::event::KeyCode::Down | ::crossterm::event::KeyCode::Char('j'),
            modifiers: ::crossterm::event::KeyModifiers::NONE,
            kind: ::crossterm::event::KeyEventKind::Press,
            state: ::crossterm::event::KeyEventState::NONE,
        }
        | ::crossterm::event::KeyEvent {
            code: ::crossterm::event::KeyCode::Char('j'),
            modifiers: ::crossterm::event::KeyModifiers::SHIFT,
            kind: ::crossterm::event::KeyEventKind::Press,
            state: ::crossterm::event::KeyEventState::NONE,
        } => {
            send_evt(systems::Key::Down, &mut app.world);
            render_state.mark_dirty(match state {
                AppState::Watchlist | AppState::WatchlistPool => DirtyFlags::WATCHLIST,
                AppState::Pool => DirtyFlags::POOL_DETAIL,
                _ => DirtyFlags::ALL,
            });
        }
        ::crossterm::event::KeyEvent {
            code: ::crossterm::event::KeyCode::Left | ::crossterm::event::KeyCode::Char('h'),
            modifiers: ::crossterm::event::KeyModifiers::NONE,
            kind: ::crossterm::event::KeyEventKind::Press,
            state: ::crossterm::event::KeyEventState::NONE,
        }
        | ::crossterm::event::KeyEvent {
            code: ::crossterm::event::KeyCode::Char('h'),
            modifiers: ::crossterm::event::KeyModifiers::SHIFT,
            kind: ::crossterm::event::KeyEventKind::Press,
            state: ::crossterm::event::KeyEventState::NONE,
        } => {
            send_evt(systems::Key::Left, &mut app.world);
            render_state.mark_dirty(match state {
                AppState::Pool => DirtyFlags::POOL_DETAIL,
                _ => DirtyFlags::ALL,
            });
        }
        ::crossterm::event::KeyEvent {
            code: ::crossterm::event::KeyCode::Right | ::crossterm::event::KeyCode::Char('l'),
            modifiers: ::crossterm::event::KeyModifiers::NONE,
            kind: ::crossterm::event::KeyEventKind::Press,
            state: ::crossterm::event::KeyEventState::NONE,
        }
        | ::crossterm::event::KeyEvent {
            code: ::crossterm::event::KeyCode::Char('l'),
            modifiers: ::crossterm::event::KeyModifiers::SHIFT,
            kind: ::crossterm::event::KeyEventKind::Press,
            state: ::crossterm::event::KeyEventState::NONE,
        } => {
            send_evt(systems::Key::Right, &mut app.world);
            render_state.mark_dirty(match state {
                AppState::Pool => DirtyFlags::POOL_DETAIL,
                _ => DirtyFlags::ALL,
            });
        }
        key!(Tab) => {
            send_evt(systems::Key::Tab, &mut app.world);
            render_state.mark_dirty(match state {
                AppState::Pool => DirtyFlags::POOL_DETAIL,
                _ => DirtyFlags::ALL,
            });
        }
        key!(Enter) => {
            send_evt(systems::Key::Enter, &mut app.world);
            render_state.mark_dirty(DirtyFlags::ALL);
        }
        shift!(BackTab) => {
            send_evt(systems::Key::BackTab, &mut app.world);
            render_state.mark_dirty(match state {
                AppState::Pool => DirtyFlags::POOL_DETAIL,
                _ => DirtyFlags::ALL,
            });
        }
        _ => (),
    }
}

fn send_evt<T: Event>(evt: T, world: &mut World) {
    let mut state = SystemState::<EventWriter<T>>::new(world);
    state.get_mut(world).send(evt);
}

fn show_index(world: &mut World, index: usize) {
    let indexes = world.resource::<Carousel<[Counter; 3]>>().current();
    world.insert_resource(systems::PoolDetail(indexes[index].clone()));
    world.insert_resource(NextState(Some(AppState::WatchlistPool)));
}
