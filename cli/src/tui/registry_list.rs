use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::api::registry::RegistryInfo;

use super::App;

pub struct RegistryListWidget {
    pub items: Vec<RegistryInfo>,
    pub state: ListState,
}

impl RegistryListWidget {
    pub fn new() -> Self {
        Self {
            items: vec![],
            state: ListState::default(),
        }
    }

    pub fn set_items(&mut self, items: Vec<RegistryInfo>) {
        self.items = items;
        if !self.items.is_empty() {
            self.state.select(Some(0));
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

    pub fn selected(&self) -> Option<&RegistryInfo> {
        self.state.selected().and_then(|i| self.items.get(i))
    }
}

pub fn render(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(f.area());

    let main_area = chunks[0];
    let help_area = chunks[1];

    let items: Vec<ListItem> = app
        .registry_list
        .items
        .iter()
        .map(|r| {
            let mode_color = match r.mode.as_str() {
                "local" => Color::Green,
                "hybrid" => Color::Cyan,
                _ => Color::Blue,
            };
            let line = Line::from(vec![
                Span::styled(
                    format!("{:<20}", r.name),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(
                    format!("{:<12}", r.registry_type),
                    Style::default().fg(Color::Gray),
                ),
                Span::styled(format!("[{:<6}]", r.mode), Style::default().fg(mode_color)),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title(" BatleHub — Registries ")
                .borders(Borders::ALL),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    f.render_stateful_widget(list, main_area, &mut app.registry_list.state.clone());

    let help = Paragraph::new(" q:quit  ↑↓:navigate  Enter:select  p:publish  ?:help")
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(help, help_area);
}
