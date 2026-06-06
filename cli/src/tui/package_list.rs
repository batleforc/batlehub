use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};
use tui_input::Input;

use crate::api::package::{PackageStatus, PackageSummary};

use super::App;

pub struct PackageListWidget {
    pub items: Vec<PackageSummary>,
    pub state: ListState,
    pub total: usize,
    pub search_active: bool,
    pub search_input: Input,
}

impl PackageListWidget {
    pub fn new() -> Self {
        Self {
            items: vec![],
            state: ListState::default(),
            total: 0,
            search_active: false,
            search_input: Input::default(),
        }
    }

    pub fn set_items(&mut self, items: Vec<PackageSummary>, total: usize) {
        self.items = items;
        self.total = total;
        if !self.items.is_empty() {
            self.state.select(Some(0));
        } else {
            self.state.select(None);
        }
    }

    pub fn next(&mut self) {
        let len = self.visible_items().len();
        if len == 0 {
            return;
        }
        let i = self.state.selected().map(|i| (i + 1) % len).unwrap_or(0);
        self.state.select(Some(i));
    }

    pub fn prev(&mut self) {
        let len = self.visible_items().len();
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
        let visible = self.visible_items();
        self.state.selected().and_then(|i| visible.get(i).copied())
    }

    pub fn visible_items(&self) -> Vec<&PackageSummary> {
        let query = self.search_input.value().to_lowercase();
        if query.is_empty() {
            self.items.iter().collect()
        } else {
            self.items
                .iter()
                .filter(|p| p.name.to_lowercase().contains(&query))
                .collect()
        }
    }

    pub fn toggle_search(&mut self) {
        self.search_active = !self.search_active;
        if !self.search_active {
            self.search_input = Input::default();
        }
    }

    pub fn handle_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.search_active = false;
                self.search_input = Input::default();
            }
            _ => {
                use tui_input::backend::crossterm::EventHandler;
                self.search_input
                    .handle_event(&crossterm::event::Event::Key(key));
            }
        }
    }
}

pub fn render(f: &mut Frame, app: &App, registry: &str) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(if app.package_list.search_active { 3 } else { 0 }),
            Constraint::Length(1),
        ])
        .split(f.area());

    let main_area = chunks[0];
    let search_area = chunks[1];
    let help_area = chunks[2];

    let visible = app.package_list.visible_items();

    let items: Vec<ListItem> = visible
        .iter()
        .map(|p| {
            let (status_str, status_color) = match &p.status {
                PackageStatus::Available => ("available", Color::Green),
                PackageStatus::Blocked { .. } => ("blocked  ", Color::Red),
            };
            let line = Line::from(vec![
                Span::styled(
                    format!("{:<30}", p.name),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(
                    format!("{:<20}", p.version),
                    Style::default().fg(Color::Gray),
                ),
                Span::styled(status_str, Style::default().fg(status_color)),
                Span::styled(
                    format!("  {:>6} downloads", p.access_count),
                    Style::default().fg(Color::DarkGray),
                ),
            ]);
            ListItem::new(line)
        })
        .collect();

    let title = format!(
        " {registry} — {} / {} packages ",
        visible.len(),
        app.package_list.total
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

    f.render_stateful_widget(list, main_area, &mut app.package_list.state.clone());

    if app.package_list.search_active {
        let search_block = Block::default().title(" Search ").borders(Borders::ALL);
        let inner = search_block.inner(search_area);
        f.render_widget(search_block, search_area);
        let search_display =
            Paragraph::new(app.package_list.search_input.value()).style(Style::default());
        f.render_widget(search_display, inner);
    }

    let help = Paragraph::new(" Esc:back  ↑↓:navigate  Enter:versions  /:search  ?:help")
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(help, help_area);
}
