use ratatui::style::{Color, Modifier, Style};

pub struct Theme {
    pub header_fg: Color,
    pub border: Color,
    pub title: Color,
    pub normal: Style,
    pub dim: Style,
    pub selected: Style,
    pub header_cell: Style,
    pub modrinth: Color,
    pub curseforge: Color,
    pub ok: Color,
    pub warn: Color,
    pub err: Color,
    pub progress: Color,
    pub footer: Style,
}

impl Theme {
    pub fn dark(color_enabled: bool) -> Self {
        if !color_enabled {
            return Self::plain();
        }
        Self {
            header_fg: Color::White,
            border: Color::Gray,
            title: Color::LightCyan,
            normal: Style::default().fg(Color::White),
            dim: Style::default().fg(Color::DarkGray),
            selected: Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            header_cell: Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
            modrinth: Color::LightGreen,
            curseforge: Color::Rgb(244, 114, 182),
            ok: Color::LightGreen,
            warn: Color::Yellow,
            err: Color::LightRed,
            progress: Color::Cyan,
            footer: Style::default().fg(Color::Gray),
        }
    }

    fn plain() -> Self {
        Self {
            header_fg: Color::Reset,
            border: Color::Reset,
            title: Color::Reset,
            normal: Style::reset(),
            dim: Style::reset(),
            selected: Style::reset().add_modifier(Modifier::BOLD),
            header_cell: Style::reset().add_modifier(Modifier::BOLD),
            modrinth: Color::Reset,
            curseforge: Color::Reset,
            ok: Color::Reset,
            warn: Color::Reset,
            err: Color::Reset,
            progress: Color::Reset,
            footer: Style::reset(),
        }
    }
}
