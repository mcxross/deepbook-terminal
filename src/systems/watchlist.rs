use std::collections::HashMap;

use bevy_ecs::{
    prelude::*,
    system::{CommandQueue, InsertResource},
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::Modifier,
    widgets::{Block, Borders, Cell, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table},
    Frame,
};
use rust_decimal::Decimal;
use tokio::sync::mpsc;

use crate::{
    app::{AppState, RT, WATCHLIST},
    data::{Counter, POOLS},
    systems::WS,
    ui::styles,
    utils::{cycle, DecimalExt, Sign},
    widgets::LocalSearch,
};

use super::{Command, Key, NavFooter, PoolDetail, PopUp, LAST_DONE, WATCHLIST_TABLE};

pub fn render_watchlist(
    mut terminal: ResMut<crate::widgets::Terminal>,
    mut events: EventReader<Key>,
    command: Res<Command>,
    (state, indexes, ws): NavFooter,
    (mut search, mut watchgroup): PopUp,
    mut log_panel: Local<crate::widgets::LogPanel>,
) {
    for event in &mut events {
        match event {
            Key::Up => {
                let len = WATCHLIST.read().expect("poison").counters().len();
                let mut table = WATCHLIST_TABLE.lock().expect("poison");
                let idx = table.selected();
                table.select(cycle::prev(idx, len));
            }
            Key::Down => {
                let len = WATCHLIST.read().expect("poison").counters().len();
                let mut table = WATCHLIST_TABLE.lock().expect("poison");
                let idx = table.selected();
                table.select(cycle::next(idx, len));
            }
            Key::Left | Key::Right | Key::Tab | Key::BackTab => (),
            Key::Enter => {
                let Some(idx) = WATCHLIST_TABLE.lock().expect("poison").selected() else {
                    continue;
                };
                let counter = WATCHLIST
                    .read()
                    .expect("poison")
                    .counters()
                    .get(idx)
                    .cloned();
                if let Some(counter) = counter {
                    _ = command.0.send({
                        let mut queue = CommandQueue::default();
                        queue.push(InsertResource {
                            resource: PoolDetail(counter),
                        });
                        queue.push(InsertResource {
                            resource: NextState(Some(AppState::WatchlistPool)),
                        });
                        queue
                    });
                }
            }
        }
    }

    _ = terminal.draw(|frame| {
        let rect = frame.size();
        let top = Rect { height: 1, ..rect };
        crate::views::navbar::render(frame, top, *state.get());

        let bottom = Rect {
            y: rect.y + rect.height - 1,
            height: 1,
            ..rect
        };
        crate::views::footer::render(frame, bottom, indexes.tick(), &ws);

        let rect = Rect {
            y: rect.y + 1,
            height: rect.height - 2,
            ..rect
        };

        let chunks = Layout::default()
            .constraints([Constraint::Length(81), Constraint::Min(20)])
            .direction(Direction::Horizontal)
            .split(rect);

        watch(frame, chunks[0], true);
        banner(frame, chunks[1]);

        crate::views::popup::render(frame, rect, &mut search, &mut watchgroup);

        let log_panel_visible = crate::app::LOG_PANEL_VISIBLE.load(atomic::Ordering::Relaxed);
        if log_panel_visible {
            log_panel.set_visible(true);
            let panel_height = 15;
            let log_rect = Rect {
                x: rect.x,
                y: rect.y + rect.height.saturating_sub(panel_height),
                width: rect.width,
                height: panel_height,
            };
            log_panel.render(frame, log_rect);
        }
    });
}

pub fn watch(frame: &mut Frame, rect: Rect, full_mode: bool) {
    let (counters, group_name) = {
        let watchlist = WATCHLIST.read().expect("poison");
        (
            watchlist.counters().to_vec(),
            watchlist
                .group()
                .map_or_else(String::new, |g| format!("{} ", g.name)),
        )
    };

    let background = Block::default()
        .borders(Borders::ALL)
        .border_style(styles::border())
        .title(format!(" Watchlist ─── {group_name}[g] "));
    frame.render_widget(background, rect);

    let mut table_state = WATCHLIST_TABLE.lock().expect("poison");
    let selected = table_state.selected();

    let block_inner = rect.inner(&Margin {
        vertical: 2,
        horizontal: 0,
    });
    let table_area = Rect {
        x: block_inner.x + 2,
        y: block_inner.y,
        width: block_inner.width.saturating_sub(3),
        height: block_inner.height,
    };
    frame.render_stateful_widget(
        watch_group_table(
            &counters,
            selected,
            &mut LAST_DONE.lock().expect("poison"),
            full_mode,
        ),
        table_area,
        &mut *table_state,
    );

    let mut scrollbar_state = ScrollbarState::new(counters.len()).position(selected.unwrap_or(0));
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(None)
        .end_symbol(None);
    let scrollbar_area = Rect {
        x: block_inner.x + block_inner.width - 1,
        y: block_inner.y,
        width: 1,
        height: block_inner.height,
    };
    frame.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
}

fn banner(frame: &mut Frame, rect: Rect) {
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(styles::border()),
        rect,
    );

    frame.render_widget(
        crate::ui::assets::banner(crate::ui::styles::text()),
        crate::ui::rect::centered(0, crate::ui::assets::BANNER_HEIGHT, rect),
    );
}

pub fn watch_group_table(
    counters: &[Counter],
    selected: Option<usize>,
    last_dones: &mut HashMap<Counter, Decimal>,
    full_mode: bool,
) -> Table<'static> {
    const COLUMN_WIDTHS: [usize; 5] = [21, 10, 8, 10, 14];
    const COLUMN_WIDTHS_FULL: [Constraint; 5] = [
        Constraint::Length(21),
        Constraint::Length(10),
        Constraint::Length(8),
        Constraint::Length(10),
        Constraint::Length(14),
    ];
    const COLUMN_WIDTHS_COMPACT: [Constraint; 3] = [
        Constraint::Length(21),
        Constraint::Length(10),
        Constraint::Length(8),
    ];

    let header = {
        let mut cells = Vec::with_capacity(if full_mode { 5 } else { 3 });
        cells.push(Cell::from("POOL").style(styles::header()));
        cells.push(Cell::from("PRICE").style(styles::header()));
        cells.push(
            Cell::from(crate::ui::text::align_right("CHG", COLUMN_WIDTHS[2]))
                .style(styles::header()),
        );
        if full_mode {
            cells.push(
                Cell::from(crate::ui::text::align_right("VOL", COLUMN_WIDTHS[3]))
                    .style(styles::header()),
            );
            cells.push(Cell::from("STATUS").style(styles::header()));
        }
        Row::new(cells)
    };

    let pools = POOLS.mget(counters);
    let rows = counters
        .iter()
        .zip(pools.iter())
        .map(|(counter, pool)| {
            static EMPTY: std::sync::LazyLock<crate::data::Pool> =
                std::sync::LazyLock::new(crate::data::Pool::default);
            let pool = pool.as_deref().unwrap_or(&EMPTY);
            let quote_data = &pool.quote;

            let display_price = quote_data
                .last_done
                .or(quote_data.prev_close)
                .filter(|&p| p > Decimal::ZERO)
                .unwrap_or_default();

            let _last = last_dones.insert(counter.clone(), display_price);

            let prev_close = quote_data.prev_close.filter(|&p| p > Decimal::ZERO);
            let current_price = quote_data
                .last_done
                .or(quote_data.open)
                .filter(|&p| p > Decimal::ZERO);

            let (increase, increase_percent) = match (current_price, prev_close) {
                (Some(price), Some(prev)) => {
                    let increase = price - prev;
                    let percent = (increase / prev * Decimal::from(100)).round_dp(2);
                    (increase, percent)
                }
                _ => (Decimal::ZERO, Decimal::ZERO),
            };

            let style = styles::up(increase.sign());

            let status_label = pool.status.clone();
            let change_sign = if increase.is_sign_positive() { "" } else { "-" };
            let percent_str = if increase_percent.fract().abs() == Decimal::ZERO {
                format!("{}", increase_percent.abs().trunc())
            } else {
                format!("{}", increase_percent.abs())
            };
            let increase_percent_str = format!("{change_sign}{percent_str}%");
            let mut cells = Vec::with_capacity(if full_mode { 5 } else { 3 });
            cells.push(Cell::from(pool.display_name().to_string()));
            cells.push(Cell::from(display_price.format_quote_by_counter(counter)).style(style));
            cells.push(
                Cell::from(crate::ui::text::align_right(
                    &increase_percent_str,
                    COLUMN_WIDTHS[2],
                ))
                .style(style),
            );
            if full_mode {
                let normalized_volume = crate::openapi::quote::normalize_base_volume_u64(
                    counter.as_str(),
                    quote_data.volume,
                );
                let volume_text = crate::utils::format_volume_decimal(normalized_volume);
                cells.push(Cell::from(crate::ui::text::align_right(
                    &volume_text,
                    COLUMN_WIDTHS[3],
                )));
                cells.push(Cell::from(status_label));
            }
            Row::new(cells)
        })
        .collect::<Vec<Row<'static>>>();

    let highlight_style = selected
        .map(|i| {
            let increase = if let Some(Some(pool)) = pools.get(i) {
                let quote_data = &pool.quote;
                let display_price = quote_data
                    .last_done
                    .or(quote_data.prev_close)
                    .filter(|&p| p > Decimal::ZERO);
                let prev_close = quote_data.prev_close.filter(|&p| p > Decimal::ZERO);

                match (display_price, prev_close) {
                    (Some(price), Some(prev)) => price.cmp(&prev),
                    _ => std::cmp::Ordering::Equal,
                }
            } else {
                std::cmp::Ordering::Equal
            };
            styles::up(increase).add_modifier(Modifier::REVERSED)
        })
        .unwrap_or_default();

    Table::new(rows)
        .header(header)
        .highlight_style(highlight_style)
        .widths(if full_mode {
            &COLUMN_WIDTHS_FULL
        } else {
            &COLUMN_WIDTHS_COMPACT
        })
        .column_spacing(1)
}

pub fn exit_watchlist() {
    crate::app::LAST_STATE.store(AppState::Watchlist, std::sync::atomic::Ordering::Relaxed);
}

pub fn enter_watchlist_common(command: Res<Command>) {
    refresh_watchlist(command.0.clone());
}

pub fn exit_watchlist_common() {}
pub async fn fetch_watchlist(
    group_id: Option<u64>,
) -> anyhow::Result<(Vec<Counter>, Vec<crate::data::WatchlistGroup>)> {
    let ctx = crate::openapi::quote();

    match ctx.watchlist().await {
        Ok(watchlist) => {
            let mut groups = Vec::new();
            let mut counters = Vec::new();

            for group in watchlist {
                let group_id_u64 = group.id;

                groups.push(crate::data::WatchlistGroup {
                    id: group_id_u64,
                    name: group.name,
                });

                if let Some(filter_id) = group_id {
                    if group_id_u64 != filter_id {
                        continue;
                    }
                }

                for pool_entry in group.pools {
                    #[allow(irrefutable_let_patterns)]
                    if let Ok(counter) = pool_entry.symbol.parse() {
                        counters.push(counter);
                    }
                }
            }

            tracing::info!(
                "Fetched {} groups, {} pools total (filtered by group: {:?})",
                groups.len(),
                counters.len(),
                group_id
            );
            Ok((counters, groups))
        }
        Err(e) => Err(e),
    }
}

pub fn refresh_watchlist(update_tx: mpsc::UnboundedSender<CommandQueue>) {
    RT.get().unwrap().spawn(async move {
        let group_id = WATCHLIST.read().expect("poison").group_id;
        match fetch_watchlist(group_id).await {
            Ok((counters, groups)) => {
                let mut watchlist = WATCHLIST.write().expect("poison");
                watchlist.set_groups(groups);
                watchlist.load(counters);
            }
            Err(err) => {
                tracing::error!("fail to fetch watchlist: {err}");
                return;
            }
        }

        let counters = {
            let mut watchlist = WATCHLIST.write().expect("poison");
            watchlist.set_hidden(true);
            watchlist.set_sortby((0, 0, false));
            watchlist.counters().to_vec()
        };

        for counter in &counters {
            if POOLS.get(counter).is_none() {
                let mut pool = crate::data::Pool::new(counter.clone());
                pool.name = counter.to_string();
                POOLS.insert(pool);
            }
        }

        if !counters.is_empty() {
            let ctx = crate::openapi::quote();
            let symbols: Vec<String> = counters.iter().map(|c| c.as_str().to_string()).collect();

            match ctx.quote(&symbols).await {
                Ok(quotes) => {
                    for quote in quotes {
                        #[allow(irrefutable_let_patterns)]
                        if let Ok(counter) = quote.symbol.parse() {
                            POOLS.modify(counter, |pool| {
                                pool.update_from_quote(&quote);
                            });
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to fetch initial quotes: {}", e);
                }
            }

            match ctx.list_pools().await {
                Ok(metas) => {
                    for meta in metas {
                        #[allow(irrefutable_let_patterns)]
                        if let Ok(counter) = meta.pool_name.parse() {
                            POOLS.modify(counter, |pool| {
                                pool.name = meta.display_name();
                            });
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to fetch pool metadata: {}", e);
                }
            }
        }

        let _ = WS.remount("watchlist", &counters).await;

        WATCHLIST.write().expect("poison").refresh();
        WATCHLIST_TABLE.lock().expect("poison").select(None);

        let local_search = LocalSearch::new(
            WATCHLIST.read().expect("poison").groups().to_vec(),
            |keyword: &str, group: &crate::data::WatchlistGroup| {
                let keyword = &keyword.to_ascii_lowercase();
                group.name.to_ascii_lowercase().contains(keyword)
            },
        );
        let mut queue = CommandQueue::default();
        queue.push(InsertResource {
            resource: local_search,
        });
        _ = update_tx.send(queue);
    });
}
