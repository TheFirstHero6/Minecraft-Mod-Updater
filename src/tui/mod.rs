mod theme;

use std::io::stdout;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures::StreamExt;
use ratatui::layout::{Constraint, Direction, Layout, Margin};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap};
use ratatui::{Frame, Terminal};
use tokio::sync::oneshot;

use crate::config::Config;
use crate::curseforge::CurseForgeClient;
use crate::download;
use crate::modrinth::ModrinthClient;
use crate::resolve::{RemoteSource, ResolveStatus, ResolvedMod};
use crate::scan::{scan_mods_dir, ScannedMod};

use theme::Theme;

pub async fn run(config: Arc<Config>, scans: Vec<ScannedMod>) -> anyhow::Result<()> {
    let color_enabled = std::env::var_os("NO_COLOR").is_none();
    let theme = Theme::dark(color_enabled);
    let modrinth = Arc::new(ModrinthClient::new(&config.user_agent())?);
    let http_download = reqwest::Client::builder()
        .user_agent(config.user_agent())
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let curse = config
        .curseforge_api_key
        .as_ref()
        .filter(|k| !k.is_empty())
        .map(|k| CurseForgeClient::new(k))
        .transpose()?
        .map(Arc::new);

    let (tx_done, rx_done) = mpsc::channel::<Vec<ResolvedMod>>();
    let cfg_clone = Arc::clone(&config);
    let mr = Arc::clone(&modrinth);
    let cf = curse.clone();
    tokio::spawn(async move {
        let rows = crate::resolve::resolve_all(cfg_clone, scans, mr, cf).await;
        let _ = tx_done.send(rows);
    });

    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut rows: Vec<ResolvedMod> = Vec::new();
    let mut loading = true;
    let mut table_state = TableState::default();
    let mut spinner: usize = 0;
    let spin_chars = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    let mut status_msg: Option<String> = None;
    let mut reader = crossterm::event::EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(120));

    let result = loop {
        if loading {
            if let Ok(r) = rx_done.try_recv() {
                rows = r;
                loading = false;
                if !rows.is_empty() {
                    table_state.select(Some(0));
                }
            }
        }

        terminal.draw(|f| {
            draw(
                f,
                &theme,
                &config,
                &rows,
                loading,
                spinner,
                spin_chars,
                &mut table_state,
                status_msg.as_deref(),
            )
        })?;

        tokio::select! {
            _ = tick.tick() => {
                if loading {
                    spinner = (spinner + 1) % spin_chars.len();
                }
            }
            maybe_ev = reader.next() => {
                let Some(Ok(ev)) = maybe_ev else { continue };
                if let Event::Key(key) = ev {
                    if key.kind == KeyEventKind::Release {
                        continue;
                    }
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                        KeyCode::Char('r') if !loading => {
                            let (tx, rx) = oneshot::channel();
                            let cfg_r = Arc::clone(&config);
                            let mr_r = Arc::clone(&modrinth);
                            let cf_r = curse.clone();
                            let dir = config.mods_dir().to_path_buf();
                            tokio::spawn(async move {
                                let scan = match scan_mods_dir(&dir) {
                                    Ok(s) => s,
                                    Err(e) => {
                                        let _ = tx.send(Err(e.to_string()));
                                        return;
                                    }
                                };
                                let resolved =
                                    crate::resolve::resolve_all(cfg_r, scan, mr_r, cf_r).await;
                                let _ = tx.send(Ok(resolved));
                            });
                            match rx.await {
                                Ok(Ok(new_rows)) => {
                                    rows = new_rows;
                                    table_state.select(if rows.is_empty() { None } else { Some(0) });
                                    status_msg = Some("Refreshed.".into());
                                }
                                Ok(Err(e)) => status_msg = Some(format!("Scan error: {e}")),
                                Err(_) => status_msg = Some("Refresh cancelled.".into()),
                            }
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if !rows.is_empty() {
                                let i = table_state.selected().unwrap_or(0);
                                let n = (i + 1).min(rows.len() - 1);
                                table_state.select(Some(n));
                            }
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            if !rows.is_empty() {
                                let i = table_state.selected().unwrap_or(0);
                                let n = i.saturating_sub(1);
                                table_state.select(Some(n));
                            }
                        }
                        KeyCode::Char('d') => {
                            if let Some(i) = table_state.selected() {
                                if let Some(row) = rows.get(i) {
                                    if row.status != ResolveStatus::UpdateAvailable {
                                        status_msg =
                                            Some("No update for this row (or unknown).".into());
                                        continue;
                                    }
                                    if row.download_url.is_none() {
                                        status_msg = Some(
                                            "No download URL (third-party restriction?).".into(),
                                        );
                                        continue;
                                    }
                                    let client = http_download.clone();
                                    let cfg_d = Arc::clone(&config);
                                    let row_clone = row.clone();
                                    match download::download_mod_update(&client, &cfg_d, &row_clone)
                                        .await
                                    {
                                        Ok(_) => {
                                            status_msg = Some(format!(
                                                "Updated {}.",
                                                row_clone.scan.file_name
                                            ));
                                        }
                                        Err(e) => {
                                            status_msg = Some(format!("Download failed: {e}"));
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    };

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

#[allow(clippy::too_many_arguments)]
fn draw(
    f: &mut Frame,
    theme: &Theme,
    config: &Config,
    rows: &[ResolvedMod],
    loading: bool,
    spinner: usize,
    spin_chars: [char; 10],
    table_state: &mut TableState,
    status_msg: Option<&str>,
) {
    let full = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(full);

    let header_text = format!(
        " mod-updater v{}  │  MC {}  │  loaders: {}  │  {}",
        env!("CARGO_PKG_VERSION"),
        config.minecraft_version(),
        config.normalized_loaders().join(", "),
        ellipsize_path(config.mods_dir(), 48)
    );
    let header_block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(theme.border))
        .title(Span::styled(
            " Minecraft mod updates ",
            Style::default()
                .fg(theme.title)
                .add_modifier(Modifier::BOLD),
        ));
    let header_inner = header_block.inner(chunks[0]);
    f.render_widget(header_block, chunks[0]);
    let spin = if loading {
        format!("{} ", spin_chars[spinner])
    } else {
        "✓ ".to_string()
    };
    let progress = if loading {
        "Resolving mods against APIs…"
    } else {
        "Ready."
    };
    let header_para = Paragraph::new(Line::from(vec![
        Span::styled(
            spin,
            Style::default()
                .fg(theme.progress)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(header_text, Style::default().fg(theme.header_fg)),
        Span::raw("  "),
        Span::styled(progress, theme.dim),
    ]))
    .wrap(Wrap { trim: true });
    f.render_widget(header_para, header_inner);

    let widths = [
        Constraint::Percentage(22),
        Constraint::Percentage(10),
        Constraint::Percentage(12),
        Constraint::Percentage(10),
        Constraint::Percentage(10),
        Constraint::Percentage(36),
    ];
    let header_cells: Vec<Cell> = ["Mod", "Local", "Remote", "Source", "Status", "Note"]
        .iter()
        .map(|h| {
            Cell::from(*h).style(
                theme
                    .header_cell
                    .add_modifier(Modifier::UNDERLINED | Modifier::BOLD),
            )
        })
        .collect();
    let header = Row::new(header_cells).height(1).bottom_margin(0);

    let data_rows: Vec<Row> = rows
        .iter()
        .enumerate()
        .map(|(idx, r)| {
            let local = r.local_version.as_str();
            let remote = r.remote_version.as_deref().unwrap_or("—");
            let src = match r.source {
                Some(RemoteSource::Modrinth) => {
                    Span::styled("Modrinth", Style::default().fg(theme.modrinth))
                }
                Some(RemoteSource::CurseForge) => {
                    Span::styled("CurseForge", Style::default().fg(theme.curseforge))
                }
                None => Span::styled("—", theme.dim),
            };
            let (st_label, st_color) = status_style(r.status, theme);
            let note = r
                .detail
                .as_deref()
                .or(r.project_label.as_deref())
                .unwrap_or("");
            let name = ellipsize(&r.display_name, 28);
            let cells = vec![
                Cell::from(name),
                Cell::from(local),
                Cell::from(remote),
                Cell::from(Line::from(vec![src])),
                Cell::from(Span::styled(st_label, Style::default().fg(st_color))),
                Cell::from(ellipsize(note, 42)),
            ];
            let mut row = Row::new(cells).height(1);
            if table_state.selected() == Some(idx) {
                row = row.style(theme.selected);
            } else if idx % 2 == 1 {
                row = row.style(theme.dim);
            }
            row
        })
        .collect();

    let table_block = Block::default()
        .title(Span::styled(" Mods ", Style::default().fg(theme.title)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .padding(ratatui::widgets::Padding::uniform(1));
    let inner = table_block.inner(chunks[1]);
    f.render_widget(table_block, chunks[1]);

    let table = Table::new(data_rows, widths)
        .header(header)
        .column_spacing(1)
        .row_highlight_style(theme.selected);
    f.render_stateful_widget(table, inner, table_state);

    let footer_line = if let Some(s) = status_msg {
        Line::from(vec![
            Span::styled("ℹ ", Style::default().fg(theme.progress)),
            Span::styled(s, theme.normal),
        ])
    } else {
        Line::from(vec![
            Span::styled("j/k ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("move  ", theme.footer),
            Span::styled("d ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("download update  ", theme.footer),
            Span::styled("r ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("refresh  ", theme.footer),
            Span::styled("q ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("quit", theme.footer),
        ])
    };
    let footer = Paragraph::new(footer_line).style(theme.footer);
    f.render_widget(
        footer,
        chunks[2].inner(Margin {
            horizontal: 1,
            vertical: 0,
        }),
    );
}

fn status_style(status: ResolveStatus, theme: &Theme) -> (&'static str, ratatui::style::Color) {
    match status {
        ResolveStatus::Pending => ("…", theme.progress),
        ResolveStatus::Resolving => ("…", theme.progress),
        ResolveStatus::UpToDate => ("up to date", theme.ok),
        ResolveStatus::UpdateAvailable => ("update", theme.warn),
        ResolveStatus::Unknown => ("unknown", Color::DarkGray),
        ResolveStatus::Error => ("error", theme.err),
    }
}

fn ellipsize(s: &str, max_chars: usize) -> String {
    let t = s.trim();
    if t.chars().count() <= max_chars {
        return t.to_string();
    }
    let take = max_chars.saturating_sub(1);
    t.chars().take(take).collect::<String>() + "…"
}

fn ellipsize_path(p: &std::path::Path, max_chars: usize) -> String {
    ellipsize(&p.display().to_string(), max_chars)
}
