use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};
use tui_input::backend::crossterm::EventHandler;
use tui_input::Input;

use crate::api::auth::parse_oidc_paste;
use crate::config::ConfigFile;

use super::App;

#[derive(Debug, Clone, PartialEq)]
pub enum LoginMethod {
    StaticToken,
    Oidc,
    Kubernetes,
}

pub struct LoginWidget {
    pub method: LoginMethod,
    pub token_input: Input,
    pub path_input: Input,
    /// OIDC authorization URL fetched when switching to the OIDC tab.
    pub oidc_url: Option<String>,
    /// Feedback message shown in the footer.
    pub status: Option<String>,
}

impl LoginWidget {
    pub fn new() -> Self {
        Self {
            method: LoginMethod::StaticToken,
            token_input: Input::default(),
            path_input: Input::default(),
            oidc_url: None,
            status: None,
        }
    }

    /// Save the current input to the default profile in the config file.
    /// Returns `true` on success (caller should navigate back).
    pub fn save_to_config(&mut self) -> bool {
        let mut cfg = match ConfigFile::load() {
            Ok(c) => c,
            Err(e) => {
                self.status = Some(format!("Failed to load config: {e}"));
                return false;
            }
        };

        match self.method {
            LoginMethod::StaticToken => {
                let token = self.token_input.value().to_string();
                if token.is_empty() {
                    self.status = Some("Token cannot be empty.".into());
                    return false;
                }
                cfg.default.token = Some(token);
                cfg.default.oidc_refresh_token = None;
                cfg.default.oidc_expires_at = None;
                cfg.default.kubernetes_token_path = None;
            }
            LoginMethod::Oidc => {
                let raw = self.token_input.value().trim().to_string();
                if raw.is_empty() {
                    self.status = Some("Paste the token or the full redirect URL.".into());
                    return false;
                }
                let (access_token, refresh_token, expires_at) = parse_oidc_paste(&raw);
                cfg.default.token = Some(access_token);
                cfg.default.oidc_refresh_token = refresh_token;
                cfg.default.oidc_expires_at = expires_at;
                cfg.default.kubernetes_token_path = None;
            }
            LoginMethod::Kubernetes => {
                let path = self.path_input.value().trim().to_string();
                if path.is_empty() {
                    self.status = Some("Token path cannot be empty.".into());
                    return false;
                }
                cfg.default.kubernetes_token_path = Some(path);
                cfg.default.token = None;
                cfg.default.oidc_refresh_token = None;
                cfg.default.oidc_expires_at = None;
            }
        }

        match cfg.save() {
            Ok(()) => true,
            Err(e) => {
                self.status = Some(format!("Failed to save config: {e}"));
                false
            }
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        match self.method {
            LoginMethod::StaticToken | LoginMethod::Oidc => {
                self.token_input
                    .handle_event(&crossterm::event::Event::Key(key));
            }
            LoginMethod::Kubernetes => {
                self.path_input
                    .handle_event(&crossterm::event::Event::Key(key));
            }
        }
    }
}

pub fn render(f: &mut Frame, app: &App) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // tab bar
            Constraint::Min(0),    // content
            Constraint::Length(1), // footer
        ])
        .split(f.area());

    let tab_area = outer[0];
    let content_area = outer[1];
    let footer_area = outer[2];

    // ── Tab bar ────────────────────────────────────────────────────────────────
    let active = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
    let inactive = Style::default().fg(Color::DarkGray);

    let tabs = Line::from(vec![
        Span::raw("  "),
        Span::styled(
            "[1] Static Token",
            if app.login.method == LoginMethod::StaticToken {
                active
            } else {
                inactive
            },
        ),
        Span::raw("   "),
        Span::styled(
            "[2] OIDC",
            if app.login.method == LoginMethod::Oidc {
                active
            } else {
                inactive
            },
        ),
        Span::raw("   "),
        Span::styled(
            "[3] Kubernetes",
            if app.login.method == LoginMethod::Kubernetes {
                active
            } else {
                inactive
            },
        ),
    ]);
    let tab_bar =
        Paragraph::new(tabs).block(Block::default().title(" Login ").borders(Borders::ALL));
    f.render_widget(tab_bar, tab_area);

    // ── Content ────────────────────────────────────────────────────────────────
    match app.login.method {
        LoginMethod::StaticToken => {
            let lines = vec![
                Line::from(""),
                Line::from(Span::styled(
                    "Enter your static bearer token:",
                    Style::default().fg(Color::Gray),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::raw("> "),
                    Span::styled(
                        app.login.token_input.value(),
                        Style::default().fg(Color::White),
                    ),
                    Span::styled("_", Style::default().add_modifier(Modifier::SLOW_BLINK)),
                ]),
            ];
            let block = Paragraph::new(lines).block(Block::default().borders(Borders::ALL));
            f.render_widget(block, content_area);
        }
        LoginMethod::Oidc => {
            let url_line = app
                .login
                .oidc_url
                .as_deref()
                .unwrap_or("(fetching OIDC URL — press 2 again if it does not appear)");
            let lines = vec![
                Line::from(""),
                Line::from(Span::styled(
                    "Open this URL in your browser:",
                    Style::default().fg(Color::Gray),
                )),
                Line::from(""),
                Line::from(Span::styled(url_line, Style::default().fg(Color::Cyan))),
                Line::from(""),
                Line::from(Span::styled(
                    "Then paste the oidc_access_token value (or the full redirect URL):",
                    Style::default().fg(Color::Gray),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::raw("> "),
                    Span::styled(
                        app.login.token_input.value(),
                        Style::default().fg(Color::White),
                    ),
                    Span::styled("_", Style::default().add_modifier(Modifier::SLOW_BLINK)),
                ]),
            ];
            let block = Paragraph::new(lines)
                .block(Block::default().borders(Borders::ALL))
                .wrap(Wrap { trim: false });
            f.render_widget(block, content_area);
        }
        LoginMethod::Kubernetes => {
            let lines = vec![
                Line::from(""),
                Line::from(Span::styled(
                    "Path to the Kubernetes service account token file:",
                    Style::default().fg(Color::Gray),
                )),
                Line::from(Span::styled(
                    "(default: /var/run/secrets/kubernetes.io/serviceaccount/token)",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::raw("> "),
                    Span::styled(
                        app.login.path_input.value(),
                        Style::default().fg(Color::White),
                    ),
                    Span::styled("_", Style::default().add_modifier(Modifier::SLOW_BLINK)),
                ]),
            ];
            let block = Paragraph::new(lines).block(Block::default().borders(Borders::ALL));
            f.render_widget(block, content_area);
        }
    }

    // ── Footer ─────────────────────────────────────────────────────────────────
    let footer_text = if let Some(ref msg) = app.login.status {
        msg.as_str()
    } else {
        " Enter:save  Esc:back  1/2/3:switch method"
    };
    let footer_style = if app.login.status.is_some() {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    f.render_widget(Paragraph::new(footer_text).style(footer_style), footer_area);
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Option<ShouldGoBack> {
    match key.code {
        KeyCode::Esc => return Some(ShouldGoBack),
        KeyCode::Char('1') => {
            app.login.method = LoginMethod::StaticToken;
            app.login.status = None;
        }
        KeyCode::Char('3') => {
            app.login.method = LoginMethod::Kubernetes;
            app.login.status = None;
        }
        KeyCode::Enter => {
            if app.login.save_to_config() {
                return Some(ShouldGoBack);
            }
        }
        _ => app.login.handle_key(key),
    }
    None
}

/// Marker returned when the login screen wants to navigate back.
pub struct ShouldGoBack;
