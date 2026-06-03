use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::api::package::{PackageStatus, PackageSummary};

use super::App;

pub struct PackageDetailWidget {
    pub items: Vec<PackageSummary>,
    pub state: ListState,
}

impl PackageDetailWidget {
    pub fn new() -> Self {
        Self {
            items: vec![],
            state: ListState::default(),
        }
    }

    pub fn set_items(&mut self, items: Vec<PackageSummary>) {
        self.items = items;
        if !self.items.is_empty() {
            self.state.select(Some(0));
        } else {
            self.state.select(None);
        }
    }

    pub fn next(&mut self) {
        let len = self.items.len();
        if len == 0 {
            return;
        }
        let i = self.state.selected().map(|i| (i + 1) % len).unwrap_or(0);
        self.state.select(Some(i));
    }

    pub fn prev(&mut self) {
        let len = self.items.len();
        if len == 0 {
            return;
        }
        let i = self
            .state
            .selected()
            .map(|i| if i == 0 { len - 1 } else { i - 1 })
            .unwrap_or(0);
        self.state.select(Some(i));
    }

    pub fn selected(&self) -> Option<&PackageSummary> {
        self.state.selected().and_then(|i| self.items.get(i))
    }
}

pub fn render(f: &mut Frame, app: &App, registry: &str, name: &str) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(f.area());

    let main_area = chunks[0];
    let help_area = chunks[1];

    let items: Vec<ListItem> = app
        .package_detail
        .items
        .iter()
        .map(|p| {
            let (status_str, status_color) = match &p.status {
                PackageStatus::Available => ("✓ available", Color::Green),
                PackageStatus::Blocked { reason } => {
                    let _ = reason;
                    ("✗ blocked  ", Color::Red)
                }
            };
            let reason_suffix = match &p.status {
                PackageStatus::Blocked { reason } => format!("  ({})", reason),
                _ => String::new(),
            };
            let line = Line::from(vec![
                Span::styled(
                    format!("{:<20}", p.version),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(status_str, Style::default().fg(status_color)),
                Span::styled(reason_suffix, Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("   {:>8} dl", p.access_count),
                    Style::default().fg(Color::DarkGray),
                ),
            ]);
            ListItem::new(line)
        })
        .collect();

    let title = format!(
        " {registry} > {name} — {} version(s) ",
        app.package_detail.items.len()
    );
    let list = List::new(items)
        .block(Block::default().title(title).borders(Borders::ALL))
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    f.render_stateful_widget(list, main_area, &mut app.package_detail.state.clone());

    let help = Paragraph::new(" Esc:back  ↑↓:navigate  y:yank  u:unyank  ?:help")
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(help, help_area);
}
