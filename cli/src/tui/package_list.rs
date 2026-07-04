use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};
use tui_input::Input;

use crate::api::package::{PackageStatus, PackageSummary};

use super::list_nav::{select_next, select_prev, ListNav};
use super::App;

pub struct PackageListWidget {
    pub nav: ListNav<PackageSummary>,
    pub total: usize,
    pub search_active: bool,
    pub search_input: Input,
}

impl PackageListWidget {
    pub fn new() -> Self {
        Self {
            nav: ListNav::new(),
            total: 0,
            search_active: false,
            search_input: Input::default(),
        }
    }

    pub fn set_items(&mut self, items: Vec<PackageSummary>, total: usize) {
        self.nav.set_items(items);
        self.total = total;
    }

    // `next`/`prev`/`selected` can't delegate to `ListNav`'s own methods: they
    // navigate the search-filtered `visible_items()`, not the full `nav.items`,
    // so they drive `nav.state` directly at the filtered length instead.

    pub fn next(&mut self) {
        let len = self.visible_items().len();
        select_next(&mut self.nav.state, len);
    }

    pub fn prev(&mut self) {
        let len = self.visible_items().len();
        select_prev(&mut self.nav.state, len);
    }

    pub fn selected(&self) -> Option<&PackageSummary> {
        let visible = self.visible_items();
        self.nav
            .state
            .selected()
            .and_then(|i| visible.get(i).copied())
    }

    pub fn visible_items(&self) -> Vec<&PackageSummary> {
        let query = self.search_input.value().to_lowercase();
        if query.is_empty() {
            self.nav.items.iter().collect()
        } else {
            self.nav
                .items
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

    f.render_stateful_widget(list, main_area, &mut app.package_list.nav.state.clone());

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

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyModifiers, MediaKeyCode};
    use tui_input::backend::crossterm::EventHandler;

    fn package(name: &str) -> PackageSummary {
        PackageSummary {
            registry: "npm-proxy".to_owned(),
            name: name.to_owned(),
            version: "1.0.0".to_owned(),
            artifact: None,
            status: PackageStatus::Available,
            access_count: 0,
        }
    }

    #[test]
    fn set_items_selects_first_when_non_empty() {
        let mut w = PackageListWidget::new();
        w.set_items(vec![package("left-pad"), package("right-pad")], 2);
        assert_eq!(w.nav.state.selected(), Some(0));
        assert_eq!(w.total, 2);
        assert_eq!(w.selected().unwrap().name, "left-pad");
    }

    #[test]
    fn set_items_empty_clears_selection() {
        let mut w = PackageListWidget::new();
        w.set_items(vec![package("left-pad")], 1);
        assert_eq!(w.nav.state.selected(), Some(0));

        w.set_items(vec![], 0);
        assert_eq!(w.nav.state.selected(), None);
        assert!(w.selected().is_none());
    }

    #[test]
    fn next_and_prev_wrap_around() {
        let mut w = PackageListWidget::new();
        w.set_items(vec![package("left-pad"), package("right-pad")], 2);

        w.next();
        assert_eq!(w.nav.state.selected(), Some(1));
        w.next();
        assert_eq!(w.nav.state.selected(), Some(0));

        w.prev();
        assert_eq!(w.nav.state.selected(), Some(1));
    }

    #[test]
    fn next_and_prev_on_empty_list_are_noops() {
        let mut w = PackageListWidget::new();
        w.next();
        w.prev();
        assert_eq!(w.nav.state.selected(), None);
    }

    #[test]
    fn visible_items_filters_case_insensitively_by_name() {
        let mut w = PackageListWidget::new();
        w.set_items(vec![package("left-pad"), package("right-pad")], 2);

        for c in "RIGHT".chars() {
            w.search_input
                .handle_event(&crossterm::event::Event::Key(KeyEvent::new(
                    KeyCode::Char(c),
                    KeyModifiers::NONE,
                )));
        }

        let visible = w.visible_items();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].name, "right-pad");
    }

    #[test]
    fn toggle_search_resets_input_when_deactivated() {
        let mut w = PackageListWidget::new();
        w.toggle_search();
        assert!(w.search_active);

        w.search_input
            .handle_event(&crossterm::event::Event::Key(KeyEvent::new(
                KeyCode::Char('x'),
                KeyModifiers::NONE,
            )));
        assert_eq!(w.search_input.value(), "x");

        w.toggle_search();
        assert!(!w.search_active);
        assert_eq!(w.search_input.value(), "");
    }

    #[test]
    fn handle_search_key_esc_clears_and_deactivates() {
        let mut w = PackageListWidget::new();
        w.toggle_search();
        w.handle_search_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        assert_eq!(w.search_input.value(), "a");

        w.handle_search_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!w.search_active);
        assert_eq!(w.search_input.value(), "");
    }

    #[test]
    fn handle_search_key_other_key_is_forwarded_to_input() {
        let mut w = PackageListWidget::new();
        w.toggle_search();
        w.handle_search_key(KeyEvent::new(
            KeyCode::Media(MediaKeyCode::Play),
            KeyModifiers::NONE,
        ));
        assert!(w.search_active);
        assert_eq!(w.search_input.value(), "");
    }
}
