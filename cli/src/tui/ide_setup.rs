use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::api::ide::IdeSetup;

use super::list_nav::ListNav;
use super::App;

/// TUI screen that lists editors detected in the current environment and, for
/// the selected one, shows how to point its extension/plugin ecosystem at a
/// BatleHub registry. Mirrors [`super::setup_wizard`], but keyed off the
/// running editor rather than on-disk project manifests.
#[derive(Default)]
pub struct IdeSetupWidget {
    pub nav: ListNav<IdeSetup>,
}

impl IdeSetupWidget {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_items(&mut self, items: Vec<IdeSetup>) {
        self.nav.set_items(items);
    }
}

pub fn render(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(f.area());

    let main_area = chunks[0];
    let footer_area = chunks[1];

    if app.ide_setup.nav.items.is_empty() {
        let msg = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "No IDE detected in this environment.",
                Style::default().fg(Color::Yellow),
            )),
            Line::from(""),
            Line::from("Detection looks for VS Code / VSCodium and JetBrains IDEs via:"),
            Line::from("  · $TERM_PROGRAM / VSCODE_* / JetBrains terminal variables"),
            Line::from("  · ~/.config/{Code,VSCodium,JetBrains} config directories"),
            Line::from("  · a ./.idea project directory"),
            Line::from(""),
            Line::from(Span::styled(
                "Run the TUI from inside your editor's integrated terminal.",
                Style::default().fg(Color::DarkGray),
            )),
        ])
        .block(
            Block::default()
                .title(" IDE Setup — detect editor ")
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: false });
        f.render_widget(msg, main_area);
    } else {
        let h_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(28), Constraint::Min(0)])
            .split(main_area);

        let list_area = h_chunks[0];
        let detail_area = h_chunks[1];

        // Left: detected editors
        let items: Vec<ListItem> = app
            .ide_setup
            .nav
            .items
            .iter()
            .map(|d| {
                let marker = if d.registry_configured { "" } else { " *" };
                ListItem::new(Line::from(Span::styled(
                    format!("{}{marker}", d.kind.label()),
                    Style::default().fg(Color::White),
                )))
            })
            .collect();

        let list = List::new(items)
            .block(Block::default().title(" Detected ").borders(Borders::ALL))
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ");

        f.render_stateful_widget(list, list_area, &mut app.ide_setup.nav.state.clone());

        // Right: setup instructions for the selected editor
        let detail_text: Vec<Line> = if let Some(det) = app.ide_setup.nav.selected() {
            det.instructions
                .lines()
                .map(|l| Line::from(Span::raw(l)))
                .collect()
        } else {
            vec![Line::from(Span::styled(
                "Select an editor on the left",
                Style::default().fg(Color::DarkGray),
            ))]
        };

        let detail = Paragraph::new(detail_text)
            .block(
                Block::default()
                    .title(" Setup instructions ")
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: false });
        f.render_widget(detail, detail_area);
    }

    let footer =
        Paragraph::new(" ↑↓: select  Esc: back  ?: help   (* = registry not configured yet)")
            .style(Style::default().fg(Color::DarkGray));
    f.render_widget(footer, footer_area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ide::IdeKind;

    fn setup(kind: IdeKind, configured: bool) -> IdeSetup {
        IdeSetup {
            kind,
            registry_type: "openvsx",
            registry_name: "ovsx".to_owned(),
            registry_configured: configured,
            detected_via: "test".to_owned(),
            instructions: format!("setup {}", kind.label()),
        }
    }

    #[test]
    fn set_items_selects_first() {
        let mut w = IdeSetupWidget::new();
        w.set_items(vec![
            setup(IdeKind::VsCode, true),
            setup(IdeKind::JetBrains, false),
        ]);
        assert_eq!(w.nav.state.selected(), Some(0));
        assert_eq!(w.nav.selected().unwrap().kind, IdeKind::VsCode);
    }

    #[test]
    fn set_items_empty_leaves_selection_unset() {
        let mut w = IdeSetupWidget::new();
        w.set_items(vec![]);
        assert_eq!(w.nav.state.selected(), None);
        assert!(w.nav.selected().is_none());
    }

    #[test]
    fn next_prev_wrap_around() {
        let mut w = IdeSetupWidget::new();
        w.set_items(vec![
            setup(IdeKind::VsCodium, true),
            setup(IdeKind::VsCode, true),
            setup(IdeKind::JetBrains, true),
        ]);
        w.nav.next();
        assert_eq!(w.nav.state.selected(), Some(1));
        w.nav.next();
        w.nav.next();
        assert_eq!(w.nav.state.selected(), Some(0));
        w.nav.prev();
        assert_eq!(w.nav.state.selected(), Some(2));
    }
}
