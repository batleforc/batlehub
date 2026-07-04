use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use crate::api::registry::RegistryInfo;

use super::list_nav::ListNav;
use super::App;

#[derive(Default)]
pub struct RegistryListWidget {
    pub nav: ListNav<RegistryInfo>,
}

impl RegistryListWidget {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_items(&mut self, items: Vec<RegistryInfo>) {
        self.nav.set_items(items);
    }

    pub fn next(&mut self) {
        self.nav.next();
    }

    pub fn prev(&mut self) {
        self.nav.prev();
    }

    pub fn selected(&self) -> Option<&RegistryInfo> {
        self.nav.selected()
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
        .nav
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

    f.render_stateful_widget(list, main_area, &mut app.registry_list.nav.state.clone());

    let help = Paragraph::new(
        " q:quit  ↑↓:navigate  Enter:select  p:publish  s:setup wizard  L:login  ?:help",
    )
    .style(Style::default().fg(Color::DarkGray));
    f.render_widget(help, help_area);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry(name: &str) -> RegistryInfo {
        RegistryInfo {
            name: name.to_owned(),
            registry_type: "npm".to_owned(),
            mode: "proxy".to_owned(),
        }
    }

    #[test]
    fn set_items_selects_first_when_non_empty() {
        let mut w = RegistryListWidget::new();
        w.set_items(vec![registry("a"), registry("b")]);
        assert_eq!(w.nav.state.selected(), Some(0));
        assert_eq!(w.selected().unwrap().name, "a");
    }

    #[test]
    fn set_items_empty_leaves_selection_unset() {
        let mut w = RegistryListWidget::new();
        w.set_items(vec![]);
        assert_eq!(w.nav.state.selected(), None);
        assert!(w.selected().is_none());
    }

    #[test]
    fn next_and_prev_wrap_around() {
        let mut w = RegistryListWidget::new();
        w.set_items(vec![registry("a"), registry("b"), registry("c")]);
        assert_eq!(w.nav.state.selected(), Some(0));

        w.next();
        assert_eq!(w.nav.state.selected(), Some(1));
        w.next();
        assert_eq!(w.nav.state.selected(), Some(2));
        w.next();
        assert_eq!(w.nav.state.selected(), Some(0));

        w.prev();
        assert_eq!(w.nav.state.selected(), Some(2));
        w.prev();
        assert_eq!(w.nav.state.selected(), Some(1));
    }

    #[test]
    fn next_and_prev_on_empty_list_are_noops() {
        let mut w = RegistryListWidget::new();
        w.next();
        w.prev();
        assert_eq!(w.nav.state.selected(), None);
    }
}
