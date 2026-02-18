use std::sync::{atomic::Ordering, Mutex};

use atomic::Atomic;
use bevy_ecs::prelude::*;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, List, ListItem, Paragraph, Row, Table, Tabs},
    Frame,
};
use rust_decimal::Decimal;
use tokio::task::JoinHandle;

use crate::{
    app::RT,
    data::{Counter, KlineType, POOLS},
    kline::KLINES,
    openapi,
    systems::WS,
    ui::styles::{self, item},
    utils::{DecimalExt, Sign},
    widgets::Terminal,
};

use super::{Key, NavFooter, PoolDetail, PopUp, KLINE_INDEX, KLINE_TYPE};

const EMPTY_PLACEHOLDER: &str = "--";

static REFRESH_POOL_TASK: std::sync::LazyLock<Mutex<Option<JoinHandle<()>>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));
static REFRESH_EXECUTING: Atomic<bool> = Atomic::new(false);

struct RefreshGuard;

impl RefreshGuard {
    fn try_acquire() -> Option<Self> {
        if REFRESH_EXECUTING.swap(true, Ordering::Relaxed) {
            None
        } else {
            Some(RefreshGuard)
        }
    }
}

impl Drop for RefreshGuard {
    fn drop(&mut self) {
        REFRESH_EXECUTING.store(false, Ordering::Relaxed);
    }
}

pub fn render_pool(
    mut terminal: ResMut<Terminal>,
    mut events: EventReader<Key>,
    pool: Res<PoolDetail>,
    (state, indexes, ws): NavFooter,
    (mut search, mut watchgroup): PopUp,
    mut last_choose: Local<Counter>,
    mut log_panel: Local<crate::widgets::LogPanel>,
) {
    if *last_choose != pool.0 {
        if !last_choose.is_empty() {
            refresh_pool_debounced(pool.0.clone());
        }
        *last_choose = pool.0.clone();
    }

    for event in &mut events {
        match event {
            Key::Left => {
                _ = KLINE_INDEX.fetch_update(Ordering::Acquire, Ordering::Relaxed, |old| {
                    Some(old.saturating_add(1))
                });
            }
            Key::Right => {
                _ = KLINE_INDEX.fetch_update(Ordering::Acquire, Ordering::Relaxed, |old| {
                    Some(old.saturating_sub(1))
                });
            }
            Key::Tab => {
                _ = KLINE_TYPE.fetch_update(Ordering::Acquire, Ordering::Relaxed, |kline_type| {
                    Some(kline_type.next())
                });
            }
            Key::BackTab => {
                _ = KLINE_TYPE.fetch_update(Ordering::Acquire, Ordering::Relaxed, |kline_type| {
                    Some(kline_type.prev())
                });
            }
            Key::Enter | Key::Up | Key::Down => {}
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

        pool_detail(
            frame,
            rect,
            &pool.0,
            KLINE_TYPE.load(Ordering::Relaxed),
            KLINE_INDEX.load(Ordering::Relaxed),
        );
        crate::views::popup::render(frame, rect, &mut search, &mut watchgroup);

        let log_panel_visible = crate::app::LOG_PANEL_VISIBLE.load(Ordering::Relaxed);
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

pub(crate) fn pool_detail(
    frame: &mut Frame,
    rect: Rect,
    counter: &Counter,
    kline_type: KlineType,
    selected: usize,
) {
    fn price_spans(data: &crate::data::QuoteData, counter: &Counter) -> Vec<Span<'static>> {
        let display_price = data
            .last_done
            .or(data.prev_close)
            .filter(|&p| p > Decimal::ZERO);

        let prev_close = data.prev_close.filter(|&p| p > Decimal::ZERO);

        let (price_str, increase, increase_percent) = match (display_price, prev_close) {
            (Some(price), Some(prev)) => {
                let increase = price - prev;
                (
                    price.format_quote_by_counter(counter),
                    increase.format_quote_by_counter(counter),
                    (increase / prev).format_percent(),
                )
            }
            (Some(price), None) => (
                price.format_quote_by_counter(counter),
                EMPTY_PLACEHOLDER.to_string(),
                EMPTY_PLACEHOLDER.to_string(),
            ),
            _ => (
                EMPTY_PLACEHOLDER.to_string(),
                EMPTY_PLACEHOLDER.to_string(),
                EMPTY_PLACEHOLDER.to_string(),
            ),
        };

        let trend_style = styles::up(increase.sign());
        vec![
            Span::raw(" "),
            Span::styled(price_str, trend_style),
            Span::raw(" ("),
            Span::styled(format!("{increase_percent}, {increase}"), trend_style),
            Span::raw(") "),
        ]
    }

    let Some(pool) = POOLS.get(counter) else {
        return;
    };

    let mut titles = vec![Span::styled(
        format!(
            " {} ({})",
            pool.display_name(),
            counter.as_str().replace('_', "/")
        ),
        styles::primary(),
    )];
    titles.extend(price_spans(&pool.quote, counter));

    let detail_container = Block::default()
        .title(Line::from(titles))
        .borders(Borders::ALL)
        .border_style(styles::border());

    frame.render_widget(detail_container, rect);

    let fmt_decimal = |opt: Option<Decimal>| -> String {
        opt.map_or_else(
            || EMPTY_PLACEHOLDER.to_string(),
            |d| d.format_quote_by_counter(counter),
        )
    };

    let price_item = |label: String, price_opt: Option<Decimal>| -> ListItem<'static> {
        let prev_close = pool.quote.prev_close.filter(|&p| p > Decimal::ZERO);
        let price = price_opt.filter(|&p| p > Decimal::ZERO);

        match (price, prev_close) {
            (Some(p), Some(prev)) => {
                let price_str = p.format_quote_by_counter(counter);
                let cmp = p.cmp(&prev);
                let style = styles::up(cmp);
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{label}: "), styles::label()),
                    Span::styled(price_str, style),
                ]))
            }
            (Some(p), None) => {
                let price_str = p.format_quote_by_counter(counter);
                item(label, price_str)
            }
            (None, Some(prev)) => {
                let price_str = prev.format_quote_by_counter(counter);
                item(label, price_str)
            }
            _ => item(label, EMPTY_PLACEHOLDER),
        }
    };

    let symbol = counter.as_str();
    let format_trade_volume_u64 = |val: u64| -> String {
        if val == 0 {
            EMPTY_PLACEHOLDER.to_string()
        } else {
            let normalized = openapi::quote::normalize_base_volume_u64(symbol, val);
            crate::utils::format_volume_decimal(normalized)
        }
    };
    let format_depth_volume_i64 = |val: i64| -> String {
        if val == 0 {
            EMPTY_PLACEHOLDER.to_string()
        } else {
            let normalized = openapi::quote::normalize_base_volume_i64(symbol, val);
            crate::utils::format_volume_decimal(normalized)
        }
    };
    let average_price = {
        let normalized_volume =
            crate::openapi::quote::normalize_base_volume_u64(symbol, pool.quote.volume);
        if normalized_volume > Decimal::ZERO && pool.quote.quote_volume > Decimal::ZERO {
            Some(pool.quote.quote_volume / normalized_volume)
        } else {
            pool.quote.last_done.or(pool.quote.open)
        }
    };

    let column0 = vec![
        ListItem::new(" "),
        item("Status".to_string(), pool.status.clone()),
        ListItem::new(" "),
        price_item("Open".to_string(), pool.quote.open),
        item("Reference".to_string(), fmt_decimal(pool.quote.prev_close)),
        ListItem::new(" "),
        price_item("High".to_string(), pool.quote.high),
        price_item("Low".to_string(), pool.quote.low),
        item("Average".to_string(), fmt_decimal(average_price)),
        ListItem::new(" "),
        item(
            "Volume".to_string(),
            format_trade_volume_u64(pool.quote.volume),
        ),
        item(
            "Quote Vol".to_string(),
            crate::ui::text::unit(pool.quote.quote_volume, 2),
        ),
        ListItem::new(" "),
    ];

    let column_height = column0.len() as u16;

    let block_inner = rect.inner(&Margin {
        vertical: 1,
        horizontal: 0,
    });
    let inner_rect = Rect {
        x: block_inner.x + 2,
        y: block_inner.y,
        width: block_inner.width.saturating_sub(3),
        height: block_inner.height,
    };
    let chunks = Layout::default()
        .constraints([
            Constraint::Min(19),
            Constraint::Length(1),
            Constraint::Length(column_height),
        ])
        .direction(Direction::Vertical)
        .split(inner_rect);

    let divider = Block::default()
        .borders(Borders::TOP)
        .border_style(styles::border());
    frame.render_widget(divider, chunks[1]);

    let columns_chunks = Layout::default()
        .constraints([Constraint::Ratio(2, 5), Constraint::Ratio(3, 5)])
        .direction(Direction::Horizontal)
        .split(chunks[2]);
    frame.render_widget(List::new(column0), columns_chunks[0]);

    let depth_rect = columns_chunks[1];
    frame.render_widget(
        Block::default()
            .borders(Borders::LEFT)
            .border_type(BorderType::Plain)
            .border_style(styles::border()),
        depth_rect,
    );

    if !pool.depth.bids.is_empty() || !pool.depth.asks.is_empty() {
        let block_inner = Block::default().borders(Borders::LEFT).inner(depth_rect);
        let depth_inner_rect = Rect {
            x: block_inner.x + 1,
            y: block_inner.y,
            width: block_inner.width.saturating_sub(2),
            height: block_inner.height,
        };

        let total_bid_volume: i64 = pool.depth.bids.iter().map(|d| d.volume).sum();
        let total_ask_volume: i64 = pool.depth.asks.iter().map(|d| d.volume).sum();
        let total_volume = total_bid_volume + total_ask_volume;
        let (bid_ratio, ask_ratio) = if total_volume > 0 {
            let bid_r = Decimal::from(total_bid_volume) / Decimal::from(total_volume);
            let ask_r = Decimal::from(total_ask_volume) / Decimal::from(total_volume);
            (bid_r, ask_r)
        } else {
            (Decimal::ZERO, Decimal::ZERO)
        };

        let fixed_width = 23;
        let depth_volume_width = (depth_inner_rect.width as usize)
            .saturating_sub(fixed_width)
            .max(10);

        let format_depth_row = |depth: &crate::data::Depth,
                                counter: &Counter,
                                prev_close: Option<Decimal>,
                                volume_width: usize|
         -> Row<'static> {
            let position = if depth.position < 10 {
                format!("{}   ", depth.position)
            } else {
                format!("{}  ", depth.position)
            };

            let price_cmp = prev_close.map_or(std::cmp::Ordering::Equal, |pc| depth.price.cmp(&pc));
            let price_style = styles::up(price_cmp);
            let price_str = depth.price.format_quote_by_counter(counter).clone();

            let volume_str =
                crate::ui::text::align_right(&format_depth_volume_i64(depth.volume), volume_width);

            let order_count_str =
                crate::ui::text::align_right(&format!("({})", depth.order_num.clamp(0, 999)), 6);

            Row::new(vec![
                Cell::from(position).style(styles::gray()),
                Cell::from(price_str).style(price_style),
                Cell::from(volume_str),
                Cell::from(order_count_str),
            ])
        };

        let asks_rows: Vec<_> = pool
            .depth
            .asks
            .iter()
            .take(5)
            .map(|d| format_depth_row(d, counter, pool.quote.prev_close, depth_volume_width))
            .collect();
        let asks_rows: Vec<_> = asks_rows.into_iter().rev().collect();

        let bids_rows: Vec<_> = pool
            .depth
            .bids
            .iter()
            .take(5)
            .map(|d| format_depth_row(d, counter, pool.quote.prev_close, depth_volume_width))
            .collect();

        let asks_count = asks_rows.len() as u16;
        let bids_count = bids_rows.len() as u16;
        let total_depth_height = asks_count + 1 + bids_count;
        let available_height = depth_inner_rect.height;
        let top_padding = available_height.saturating_sub(total_depth_height) / 2;

        let depth_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(top_padding),
                Constraint::Length(asks_count),
                Constraint::Length(1),
                Constraint::Length(bids_count),
                Constraint::Min(0),
            ])
            .split(depth_inner_rect);

        let table_widths = vec![
            Constraint::Length(4),
            Constraint::Length(10),
            Constraint::Length(depth_volume_width as u16),
            Constraint::Length(6),
        ];

        let asks_table = Table::new(asks_rows)
            .widths(&table_widths)
            .column_spacing(1);

        frame.render_widget(asks_table, depth_layout[1]);

        let (bull_style, bear_style) = styles::bull_bear();
        let green_color = bull_style.fg.unwrap_or(Color::Green);
        let red_color = bear_style.fg.unwrap_or(Color::Red);

        let available_width = depth_layout[2].width as usize;
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let bid_width = ((Decimal::from(available_width) * bid_ratio)
            .to_string()
            .parse::<f64>()
            .unwrap_or(0.0)
            .round() as usize)
            .min(available_width);
        let ask_width = available_width.saturating_sub(bid_width);

        let bid_label = format!(" Bid: {:.1}%", bid_ratio * Decimal::from(100));
        let ask_label = format!("Ask: {:.1}% ", ask_ratio * Decimal::from(100));

        let bid_label_len = bid_label.chars().count();
        let ask_label_len = ask_label.chars().count();

        let bid_padding = bid_width.saturating_sub(bid_label_len);
        let bid_content = format!("{}{}", bid_label, " ".repeat(bid_padding));

        let ask_padding = ask_width.saturating_sub(ask_label_len);
        let ask_content = format!("{}{}", " ".repeat(ask_padding), ask_label);

        let ratio_line = Line::from(vec![
            Span::styled(
                bid_content,
                Style::default().fg(Color::White).bg(green_color),
            ),
            Span::styled(ask_content, Style::default().fg(Color::White).bg(red_color)),
        ]);

        frame.render_widget(Paragraph::new(ratio_line), depth_layout[2]);

        let bids_table = Table::new(bids_rows)
            .widths(&table_widths)
            .column_spacing(1);

        frame.render_widget(bids_table, depth_layout[3]);
    }

    let chart_chunks = Layout::default()
        .constraints([Constraint::Ratio(2, 3), Constraint::Ratio(1, 3)])
        .direction(Direction::Horizontal)
        .split(chunks[0]);

    {
        const Y_AXIS_WIDTH: u16 = 17;

        let chart_chunks_inner = Layout::default()
            .constraints([Constraint::Length(2), Constraint::Min(20)])
            .direction(Direction::Vertical)
            .split(chart_chunks[0]);

        let selected_type_index = KlineType::iter()
            .position(|t| t == kline_type)
            .unwrap_or_default();
        let chart_tabs = Tabs::new(
            KlineType::iter()
                .map(|chart_type| {
                    Line::from(vec![
                        Span::raw(" "),
                        Span::raw(chart_type.to_string()),
                        Span::raw(" "),
                    ])
                })
                .collect(),
        )
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .select(selected_type_index);
        frame.render_widget(chart_tabs, chart_chunks_inner[0]);

        let area = chart_chunks_inner[1];
        let (width, page, _index) = area
            .width
            .checked_sub(Y_AXIS_WIDTH)
            .filter(|&v| v > 0)
            .map(|width| {
                let width = width as usize;
                (width, selected / width, selected % width)
            })
            .unwrap_or_default();
        let samples = KLINES.by_pagination(counter.clone(), kline_type, page, width);

        if samples.is_empty() {
            frame.render_widget(
                Paragraph::new("Loading...").alignment(Alignment::Center),
                area,
            );
        } else {
            let candles: Vec<cli_candlestick_chart::Candle> = samples
                .iter()
                .filter_map(|sample| {
                    let open = f64::try_from(sample.open).ok()?;
                    let high = f64::try_from(sample.high).ok()?;
                    let low = f64::try_from(sample.low).ok()?;
                    let close = f64::try_from(sample.close).ok()?;

                    if open <= 0.0 || high <= 0.0 || low <= 0.0 || close <= 0.0 {
                        return None;
                    }
                    if high < low || high < open || high < close || low > open || low > close {
                        return None;
                    }

                    Some(cli_candlestick_chart::Candle {
                        open,
                        high,
                        low,
                        close,
                        volume: openapi::quote::normalize_base_volume_u64(
                            counter.as_str(),
                            sample.amount,
                        )
                        .to_string()
                        .parse::<f64>()
                        .ok(),
                        timestamp: Some(sample.timestamp),
                    })
                })
                .collect();

            if candles.is_empty() {
                frame.render_widget(
                    Paragraph::new("Invalid K-line data format").alignment(Alignment::Center),
                    area,
                );
            } else {
                let chart_width = area.width.saturating_sub(1);
                let mut chart = cli_candlestick_chart::Chart::new_with_size(
                    candles,
                    (chart_width, area.height),
                );
                let (bull, bear) = styles::bull_bear_color();
                chart.set_bull_color(bull);
                chart.set_vol_bull_color(bull);
                chart.set_bear_color(bear);
                chart.set_vol_bear_color(bear);
                frame.render_widget(crate::widgets::Ansi(&chart.render()), area);
            }
        }
    }

    {
        let trades_area = chart_chunks[1];
        frame.render_widget(
            Block::default()
                .borders(Borders::LEFT)
                .border_type(BorderType::Plain)
                .border_style(styles::border())
                .title(" Trades "),
            trades_area,
        );

        let inner_area = Rect {
            x: trades_area.x + 2,
            y: trades_area.y + 1,
            width: trades_area.width.saturating_sub(3),
            height: trades_area.height.saturating_sub(2),
        };

        if pool.trades.is_empty() {
            frame.render_widget(
                Paragraph::new("Loading...").alignment(Alignment::Center),
                inner_area,
            );
        } else {
            let fixed_width = 21;
            let volume_width = (inner_area.width as usize)
                .saturating_sub(fixed_width)
                .max(8);

            let max_volume = pool
                .trades
                .iter()
                .map(|t| t.volume.abs())
                .max()
                .unwrap_or(1);

            let trade_rows: Vec<Row> = pool
                .trades
                .iter()
                .take(inner_area.height as usize)
                .map(|trade| {
                    let time_str = time::OffsetDateTime::from_unix_timestamp(trade.timestamp)
                        .ok()
                        .and_then(|dt| {
                            let format =
                                time::format_description::parse("[hour]:[minute]:[second]").ok()?;
                            dt.format(&format).ok()
                        })
                        .unwrap_or_else(|| "--:--:--".to_string());

                    let (price_style, direction_symbol, bg_color) = match trade.direction {
                        crate::data::TradeDirection::Up => {
                            let style = styles::up(std::cmp::Ordering::Greater);
                            (style, "↑", style.fg.unwrap_or(Color::Green))
                        }
                        crate::data::TradeDirection::Down => {
                            let style = styles::up(std::cmp::Ordering::Less);
                            (style, "↓", style.fg.unwrap_or(Color::Red))
                        }
                        crate::data::TradeDirection::Neutral => {
                            (Style::default(), " ", Color::DarkGray)
                        }
                    };

                    #[allow(clippy::cast_precision_loss)]
                    let volume_ratio = if max_volume > 0 {
                        let current_volume = trade.volume.abs() as f64;
                        let max_vol_f64 = max_volume as f64;
                        let power = 0.5;
                        let current_pow = current_volume.powf(power);
                        let max_pow = max_vol_f64.powf(power);
                        (current_pow / max_pow).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };

                    let volume_text = crate::ui::text::align_right(
                        &format_depth_volume_i64(trade.volume),
                        volume_width,
                    );

                    #[allow(
                        clippy::cast_sign_loss,
                        clippy::cast_precision_loss,
                        clippy::cast_possible_truncation
                    )]
                    let bg_width = (volume_width as f64 * volume_ratio).ceil() as usize;
                    let fg_width = volume_width.saturating_sub(bg_width);

                    let volume_chars: Vec<char> = volume_text.chars().collect();
                    let fg_part: String = volume_chars.iter().take(fg_width).collect();
                    let bg_part: String =
                        volume_chars.iter().skip(fg_width).take(bg_width).collect();

                    let volume_cell = if !fg_part.is_empty() && !bg_part.is_empty() {
                        Cell::from(Line::from(vec![
                            Span::styled(fg_part, Style::default()),
                            Span::styled(bg_part, Style::default().bg(bg_color)),
                        ]))
                    } else if !bg_part.is_empty() {
                        Cell::from(Span::styled(bg_part, Style::default().bg(bg_color)))
                    } else {
                        Cell::from(fg_part)
                    };

                    let price_str = format!("{:>8}", trade.price.format_quote_by_counter(counter));

                    Row::new(vec![
                        Cell::from(time_str).style(crate::ui::styles::label()),
                        Cell::from(direction_symbol).style(price_style),
                        Cell::from(price_str).style(price_style),
                        volume_cell,
                    ])
                })
                .collect();

            let widths = [
                Constraint::Length(9),
                Constraint::Length(1),
                Constraint::Length(8),
                Constraint::Length(volume_width as u16),
            ];
            let table = Table::new(trade_rows).widths(&widths).column_spacing(1);

            frame.render_widget(table, inner_area);
        }
    }
}

pub fn refresh_pool_debounced(counter: Counter) {
    if let Ok(mut task_guard) = REFRESH_POOL_TASK.lock() {
        if let Some(task) = task_guard.take() {
            task.abort();
        }

        let handle = RT.get().unwrap().spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;

            let Some(_guard) = RefreshGuard::try_acquire() else {
                tracing::debug!(
                    "Skipping refresh for {} - another refresh is in progress",
                    counter
                );
                return;
            };

            tracing::debug!("Starting refresh for {}", counter);

            KLINES.clear();
            let _ = WS
                .quote_detail("pool_detail", std::slice::from_ref(&counter))
                .await;
            let _ = WS
                .quote_trade("pool_detail", std::slice::from_ref(&counter))
                .await;

            let ctx = crate::openapi::quote();
            if let Ok(quotes) = ctx.quote(&[counter.to_string()]).await {
                if let Some(quote) = quotes.first() {
                    POOLS.modify(counter.clone(), |pool| {
                        pool.update_from_quote(quote);
                    });
                }
            }

            if let Ok(trades) = openapi::quote::fetch_trades(&counter.to_string(), 50).await {
                POOLS.modify(counter.clone(), |pool| {
                    pool.update_from_trades(&trades);
                });
            }

            tracing::debug!("Completed refresh for {}", counter);
        });

        *task_guard = Some(handle);
    }
}

pub fn enter_pool(counter: Res<PoolDetail>) {
    refresh_pool_debounced(counter.0.clone());
}

pub fn exit_pool() {
    KLINES.clear();
}
