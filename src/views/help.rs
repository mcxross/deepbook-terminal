use ratatui::{
    prelude::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Padding, Paragraph},
    Frame,
};

use crate::ui::styles;

const HELP_TIPS: &str = r"  - General ----------------------------------------------------------------------------------

    ?                               Show help info
    `                               Toggle debug log panel
    /                               Open pool search popup
    q, ESC                          Dismiss current window, or go back to last tab
    Enter                           Perform action for the current selection
    R                               Refresh data manually

  - Pair Detail ------------------------------------------------------------------------------

    t                               Toggle list/detail view
    TAB, Shift+TAB                  Switch kline sampling selection
    h, Left Arrow, l, Right Arrow   Switch kline sampling interval for candlestick charts

  - Watchlist --------------------------------------------------------------------------------

    G                               Switch watchlist group
    t                               Toggle pair detail view
    j, Up Arrow, k, Down Arrow      Switch watching selection
";

pub fn render(frame: &mut Frame, rect: Rect) {
    let rect = crate::ui::rect::centered(100, 40, rect);

    let mut spans = vec![
        Line::from("\n"),
        Line::styled(
            concat!("  Surflux Terminal v", env!("CARGO_PKG_VERSION")),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Line::from("\n"),
        Line::from("  https://api.surflux.dev"),
        Line::from("\n"),
    ];
    spans.extend(HELP_TIPS.split('\n').map(Line::from));
    let paragraph = Paragraph::new(spans).style(styles::popup()).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(styles::border())
            .padding(Padding::horizontal(2))
            .title(Span::styled("Help", styles::title())),
    );
    frame.render_widget(Clear, rect);
    frame.render_widget(paragraph, rect);
}
