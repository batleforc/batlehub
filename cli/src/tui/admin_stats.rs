use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};

use crate::api::admin::StatsResponse;

use super::App;

pub struct AdminStatsWidget {
    pub data: Option<StatsResponse>,
    pub loading: bool,
    pub error: Option<String>,
    pub table_state: TableState,
}

impl AdminStatsWidget {
    pub fn new() -> Self {
        Self {
            data: None,
            loading: false,
            error: None,
            table_state: TableState::default(),
        }
    }

    pub fn set_data(&mut self, v: StatsResponse) {
        self.data = Some(v);
        self.loading = false;
        self.error = None;
        self.table_state.select(None);
    }

    pub fn set_error(&mut self, e: String) {
        self.error = Some(e);
        self.loading = false;
    }
}

fn fmt_bytes(n: Option<u64>) -> String {
    match n.map(|b| b as f64) {
        None => "—".to_string(),
        Some(b) if b >= 1_073_741_824.0 => format!("{:.1} GiB", b / 1_073_741_824.0),
        Some(b) if b >= 1_048_576.0 => format!("{:.1} MiB", b / 1_048_576.0),
        Some(b) if b >= 1_024.0 => format!("{:.1} KiB", b / 1_024.0),
        Some(b) => format!("{:.0} B", b),
    }
}

fn fmt_pct(n: Option<f64>) -> String {
    match n {
        None => "—".to_string(),
        Some(p) => format!("{:.1}%", p * 100.0),
    }
}

pub fn render(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(5),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(f.area());

    // Title
    f.render_widget(
        Paragraph::new("Admin Stats  [r] refresh  [Esc] back")
            .block(Block::default().borders(Borders::BOTTOM))
            .style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        chunks[0],
    );

    let widget = &app.admin_stats;

    if widget.loading {
        f.render_widget(Paragraph::new("Loading…"), chunks[1]);
        return;
    }
    if let Some(err) = &widget.error {
        f.render_widget(
            Paragraph::new(format!("Error: {err}")).style(Style::default().fg(Color::Red)),
            chunks[1],
        );
        return;
    }
    let Some(data) = &widget.data else {
        f.render_widget(Paragraph::new("Press 'r' to refresh"), chunks[1]);
        return;
    };

    // Aggregate stats row
    let agg = &data.aggregate;
    let hit_rate = fmt_pct(agg.hit_rate);
    let hits = agg.artifact_hits.to_string();
    let misses = agg.artifact_misses.to_string();
    let cached = fmt_bytes(Some(agg.cached_bytes));
    let since = data.since_startup.to_rfc3339();

    let summary = vec![
        Line::from(vec![
            Span::styled("Hit rate: ", Style::default().fg(Color::Gray)),
            Span::styled(
                hit_rate,
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled("Hits: ", Style::default().fg(Color::Gray)),
            Span::raw(hits),
            Span::raw("  "),
            Span::styled("Misses: ", Style::default().fg(Color::Gray)),
            Span::raw(misses),
        ]),
        Line::from(vec![
            Span::styled("Cached: ", Style::default().fg(Color::Gray)),
            Span::styled(cached, Style::default().fg(Color::Yellow)),
            Span::raw("  "),
            Span::styled("Since: ", Style::default().fg(Color::Gray)),
            Span::raw(since),
        ]),
    ];
    f.render_widget(
        Paragraph::new(summary).block(Block::default().title("Aggregate").borders(Borders::ALL)),
        chunks[1],
    );

    // Per-registry table
    let header = Row::new(vec!["Registry", "Hit Rate", "Hits", "Misses", "Cached"])
        .style(Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED));

    let rows: Vec<Row> = data
        .per_registry
        .iter()
        .map(|r| {
            Row::new(vec![
                Cell::from(r.registry.clone()),
                Cell::from(fmt_pct(r.hit_rate)),
                Cell::from(r.artifact_hits.to_string()),
                Cell::from(r.artifact_misses.to_string()),
                Cell::from(fmt_bytes(r.cached_bytes)),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(30),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(12),
        ],
    )
    .header(header)
    .block(Block::default().title("Per-Registry").borders(Borders::ALL))
    .row_highlight_style(Style::default().bg(Color::DarkGray));

    f.render_stateful_widget(table, chunks[2], &mut app.admin_stats.table_state.clone());

    f.render_widget(
        Paragraph::new("[r] refresh  [Esc] back  [q] quit"),
        chunks[3],
    );
}
