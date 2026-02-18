use ratatui::{
    prelude::{Alignment, Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Paragraph, Tabs},
    Frame,
};

use crate::{app::AppState, ui::styles};

pub fn render(frame: &mut Frame, rect: Rect, state: AppState) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(rect);

    let tabs = vec![Line::from(" WATCHLIST [1] ")];

    let tabs = Tabs::new(tabs)
        .style(styles::text())
        .highlight_style(styles::text_selected())
        .divider("|")
        .select(match state {
            AppState::Watchlist | AppState::WatchlistPool | AppState::Pool => 0,
            _ => panic!("invalid state"),
        });

    // Simplified implementation: use fixed username
    let nickname = "User".to_string();
    let dark_gray_style = styles::dark_gray();
    let name = Span::styled(format!("Hi, {nickname}"), dark_gray_style);
    let help = Span::styled("Help [?]", dark_gray_style);
    let log = Span::styled("Console [`]", dark_gray_style);
    let search = Span::styled("Search [/]", dark_gray_style);
    let quit = Span::styled("Quit [^c]", dark_gray_style);
    let user_info = Paragraph::new(Line::from(vec![
        name,
        Span::styled(" | ", dark_gray_style),
        help,
        Span::styled(" ", dark_gray_style),
        log,
        Span::styled(" ", dark_gray_style),
        search,
        Span::styled(" ", dark_gray_style),
        quit,
    ]))
    .alignment(Alignment::Right);

    frame.render_widget(tabs, chunks[0]);
    frame.render_widget(user_info, chunks[1]);
}
