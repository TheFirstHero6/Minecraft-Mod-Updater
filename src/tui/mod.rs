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
use ratatui::layout::{Constraint, Direction, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Wrap,
};
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
    let spin_chars = spinner_frames();
    let mut status_msg: Option<String> = None;
    let mut show_help = false;
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
                show_help,
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
                        KeyCode::Char('?') => {
                            toggle_help(&mut show_help);
                        }
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
    spin_chars: &[char],
    table_state: &mut TableState,
    status_msg: Option<&str>,
    show_help: bool,
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
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
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
        "Checking Modrinth/CurseForge for compatible updates..."
    } else {
        "Ready. Press ? for help."
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
        Constraint::Percentage(20),
        Constraint::Percentage(10),
        Constraint::Percentage(12),
        Constraint::Percentage(10),
        Constraint::Percentage(8),
        Constraint::Percentage(10),
        Constraint::Percentage(30),
    ];
    let header_cells: Vec<Cell> = ["Mod", "Local", "Remote", "Source", "Match", "Status", "Note"]
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
            let identity = match_hint(r);
            let (st_label, st_color) = status_style(r.status, theme);
            let note = r
                .detail
                .as_deref()
                .or(r.project_label.as_deref())
                .unwrap_or_else(|| default_note(r.status));
            let name = ellipsize(&r.display_name, 28);
            let cells = vec![
                Cell::from(name),
                Cell::from(local),
                Cell::from(remote),
                Cell::from(Line::from(vec![src])),
                Cell::from(identity),
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
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border))
        .padding(ratatui::widgets::Padding::uniform(1));
    let inner = table_block.inner(chunks[1]);
    f.render_widget(table_block, chunks[1]);

    if rows.is_empty() && !loading {
        let empty = Paragraph::new(
            "No mod rows to display.\n\nCheck that your mods directory contains .jar files, then press r to rescan.",
        )
        .style(theme.normal)
        .wrap(Wrap { trim: true });
        f.render_widget(empty, inner);
    } else {
        let table = Table::new(data_rows, widths)
            .header(header)
            .column_spacing(1)
            .row_highlight_style(theme.selected);
        f.render_stateful_widget(table, inner, table_state);
    }

    let footer_line = if let Some(s) = status_msg {
        Line::from(vec![
            Span::styled("ℹ ", Style::default().fg(theme.progress)),
            Span::styled(s, theme.normal),
        ])
    } else {
        Line::from(Span::styled(default_footer_text(), theme.footer))
    };
    let footer = Paragraph::new(footer_line).style(theme.footer);
    f.render_widget(
        footer,
        chunks[2].inner(Margin {
            horizontal: 1,
            vertical: 0,
        }),
    );

    if show_help {
        let area = centered_rect(72, 56, full);
        let help = Paragraph::new(
            "Controls\n\
             - j / Down: Move selection down\n\
             - k / Up: Move selection up\n\
             - d: Download selected update\n\
             - r: Refresh scan and resolve again\n\
             - q / Esc: Quit\n\
             - ?: Toggle this help\n\n\
             Troubleshooting\n\
             - If a mod shows unknown, the host may not have an exact tag for your target MC version.\n\
             - Keep verify_after_download enabled to reject incompatible downloads.",
        )
        .block(
            Block::default()
                .title(Span::styled(" Help ", Style::default().fg(theme.title)))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.border)),
        )
        .style(theme.normal)
        .wrap(Wrap { trim: true });
        f.render_widget(Clear, area);
        f.render_widget(help, area);
    }
}

fn status_style(status: ResolveStatus, theme: &Theme) -> (&'static str, ratatui::style::Color) {
    match status {
        ResolveStatus::Pending => ("pending", theme.progress),
        ResolveStatus::Resolving => ("resolving", theme.progress),
        ResolveStatus::UpToDate => ("up to date", theme.ok),
        ResolveStatus::UpdateAvailable => ("update available", theme.warn),
        ResolveStatus::Unknown => ("needs review", Color::DarkGray),
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

fn match_hint(row: &ResolvedMod) -> Span<'static> {
    match row.identity_match {
        Some(true) => Span::styled("=", Style::default().fg(Color::Green)),
        Some(false) => {
            let suffix = row
                .remote_file_sha512
                .as_deref()
                .map(short_hash)
                .unwrap_or("≠");
            Span::styled(format!("≠ {suffix}"), Style::default().fg(Color::Yellow))
        }
        None => Span::styled("?", Style::default().fg(Color::DarkGray)),
    }
}

fn short_hash(hash: &str) -> &str {
    hash.get(..6).unwrap_or(hash)
}

fn toggle_help(show_help: &mut bool) {
    *show_help = !*show_help;
}

fn spinner_frames() -> &'static [char] {
    pick_spinner_frames(
        std::env::var_os("MOD_UPDATER_ASCII").is_some(),
        cfg!(target_os = "windows"),
    )
}

fn pick_spinner_frames(force_ascii: bool, is_windows: bool) -> &'static [char] {
    const UNICODE: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    const ASCII: [char; 4] = ['|', '/', '-', '\\'];
    if force_ascii || is_windows {
        &ASCII
    } else {
        &UNICODE
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1]);
    horizontal[1]
}

fn default_footer_text() -> &'static str {
    "? Help  |  j/k Move  |  d Download selected update  |  r Refresh scan  |  q Quit"
}

fn default_note(status: ResolveStatus) -> &'static str {
    match status {
        ResolveStatus::UpToDate => "No newer file found for the selected target.",
        ResolveStatus::UpdateAvailable => "Press d to download the selected update.",
        ResolveStatus::Unknown => "No compatible candidate found for this loader/MC target.",
        ResolveStatus::Error => "Resolution failed. Check note details.",
        ResolveStatus::Pending | ResolveStatus::Resolving => "Resolving metadata...",
    }
}

#[cfg(test)]
mod tests {
    use super::{default_footer_text, pick_spinner_frames, toggle_help};

    #[test]
    fn spinner_prefers_ascii_when_forced() {
        let frames = pick_spinner_frames(true, false);
        assert_eq!(frames, ['|', '/', '-', '\\']);
    }

    #[test]
    fn spinner_prefers_ascii_on_windows() {
        let frames = pick_spinner_frames(false, true);
        assert_eq!(frames, ['|', '/', '-', '\\']);
    }

    #[test]
    fn help_toggle_switches_state() {
        let mut show_help = false;
        toggle_help(&mut show_help);
        assert!(show_help);
        toggle_help(&mut show_help);
        assert!(!show_help);
    }

    #[test]
    fn footer_has_action_labels() {
        let footer = default_footer_text();
        assert!(footer.contains("? Help"));
        assert!(footer.contains("Download selected update"));
    }
}
