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
            header_fg: Color::Cyan,
            border: Color::DarkGray,
            title: Color::LightCyan,
            normal: Style::default().fg(Color::Gray),
            dim: Style::default().fg(Color::DarkGray),
            selected: Style::default()
                .fg(Color::Black)
                .bg(Color::LightBlue)
                .add_modifier(Modifier::BOLD),
            header_cell: Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
            modrinth: Color::Green,
            curseforge: Color::Rgb(244, 114, 182),
            ok: Color::Rgb(134, 239, 172),
            warn: Color::Rgb(251, 191, 36),
            err: Color::Rgb(248, 113, 113),
            progress: Color::LightCyan,
            footer: Style::default().fg(Color::DarkGray),
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
