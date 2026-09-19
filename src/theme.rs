use ratatui::style::Color;

/// The built-in terminal color schemes available from wtop's theme menu.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Theme {
    #[default]
    Default,
    Monochromatic,
    BlackOnWhite,
    LightTerminal,
    BlackNight,
    BrokenGray,
    Nord,
    Ocean,
    Evergreen,
    Dusk,
}

/// A semantic palette used by every part of the terminal interface.
///
/// Keeping colors named by purpose instead of using terminal colors directly
/// lets a theme change the entire dashboard coherently.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Palette {
    pub background: Color,
    pub foreground: Color,
    pub border: Color,
    pub accent: Color,
    pub muted: Color,
    pub selection_foreground: Color,
    pub selection_background: Color,
    pub good: Color,
    pub warning: Color,
    pub critical: Color,
}

impl Theme {
    pub const ALL: [Self; 10] = [
        Self::Default,
        Self::Monochromatic,
        Self::BlackOnWhite,
        Self::LightTerminal,
        Self::BlackNight,
        Self::BrokenGray,
        Self::Nord,
        Self::Ocean,
        Self::Evergreen,
        Self::Dusk,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Monochromatic => "Monochromatic",
            Self::BlackOnWhite => "Black on White",
            Self::LightTerminal => "Light Terminal",
            Self::BlackNight => "Black Night",
            Self::BrokenGray => "Broken Gray",
            Self::Nord => "Nord",
            Self::Ocean => "Ocean",
            Self::Evergreen => "Evergreen",
            Self::Dusk => "Dusk",
        }
    }

    pub fn next(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|theme| *theme == self)
            .unwrap_or_default();
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    pub fn previous(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|theme| *theme == self)
            .unwrap_or_default();
        Self::ALL[(index + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    pub fn palette(self) -> Palette {
        match self {
            Self::Default => Palette {
                background: Color::Reset,
                foreground: Color::White,
                border: Color::DarkGray,
                accent: Color::Cyan,
                muted: Color::DarkGray,
                selection_foreground: Color::White,
                selection_background: Color::Blue,
                good: Color::Green,
                warning: Color::Yellow,
                critical: Color::Red,
            },
            Self::Monochromatic => Palette {
                background: Color::Black,
                foreground: Color::Gray,
                border: Color::DarkGray,
                accent: Color::White,
                muted: Color::DarkGray,
                selection_foreground: Color::Black,
                selection_background: Color::White,
                good: Color::Gray,
                warning: Color::White,
                critical: Color::White,
            },
            Self::BlackOnWhite => Palette {
                background: Color::White,
                foreground: Color::Black,
                border: Color::DarkGray,
                accent: Color::Black,
                muted: Color::Gray,
                selection_foreground: Color::White,
                selection_background: Color::Black,
                good: Color::Black,
                warning: Color::Red,
                critical: Color::Red,
            },
            Self::LightTerminal => Palette {
                background: Color::White,
                foreground: Color::DarkGray,
                border: Color::DarkGray,
                accent: Color::LightBlue,
                muted: Color::DarkGray,
                selection_foreground: Color::White,
                selection_background: Color::LightBlue,
                good: Color::Green,
                warning: Color::Red,
                critical: Color::Red,
            },
            Self::BlackNight => Palette {
                background: Color::Black,
                foreground: Color::Gray,
                border: Color::DarkGray,
                accent: Color::LightCyan,
                muted: Color::DarkGray,
                selection_foreground: Color::Black,
                selection_background: Color::LightCyan,
                good: Color::LightGreen,
                warning: Color::LightYellow,
                critical: Color::LightRed,
            },
            Self::BrokenGray => Palette {
                background: Color::DarkGray,
                foreground: Color::White,
                border: Color::Gray,
                accent: Color::Gray,
                muted: Color::Gray,
                selection_foreground: Color::Black,
                selection_background: Color::Gray,
                good: Color::White,
                warning: Color::Gray,
                critical: Color::White,
            },
            Self::Nord => Palette {
                background: Color::Rgb(46, 52, 64),
                foreground: Color::Rgb(216, 222, 233),
                border: Color::Rgb(76, 86, 106),
                accent: Color::Rgb(136, 192, 208),
                muted: Color::Rgb(129, 161, 193),
                selection_foreground: Color::Rgb(46, 52, 64),
                selection_background: Color::Rgb(136, 192, 208),
                good: Color::Rgb(163, 190, 140),
                warning: Color::Rgb(235, 203, 139),
                critical: Color::Rgb(191, 97, 106),
            },
            Self::Ocean => Palette {
                background: Color::Rgb(15, 31, 45),
                foreground: Color::Rgb(220, 235, 245),
                border: Color::Rgb(70, 112, 140),
                accent: Color::Rgb(94, 200, 230),
                muted: Color::Rgb(139, 173, 194),
                selection_foreground: Color::Rgb(15, 31, 45),
                selection_background: Color::Rgb(94, 200, 230),
                good: Color::Rgb(115, 205, 150),
                warning: Color::Rgb(245, 195, 92),
                critical: Color::Rgb(245, 112, 112),
            },
            Self::Evergreen => Palette {
                background: Color::Rgb(20, 36, 32),
                foreground: Color::Rgb(224, 239, 226),
                border: Color::Rgb(76, 118, 98),
                accent: Color::Rgb(117, 205, 165),
                muted: Color::Rgb(154, 184, 166),
                selection_foreground: Color::Rgb(20, 36, 32),
                selection_background: Color::Rgb(117, 205, 165),
                good: Color::Rgb(150, 214, 121),
                warning: Color::Rgb(241, 202, 106),
                critical: Color::Rgb(242, 126, 126),
            },
            Self::Dusk => Palette {
                background: Color::Rgb(35, 29, 48),
                foreground: Color::Rgb(237, 229, 246),
                border: Color::Rgb(98, 82, 128),
                accent: Color::Rgb(183, 148, 244),
                muted: Color::Rgb(174, 158, 202),
                selection_foreground: Color::Rgb(35, 29, 48),
                selection_background: Color::Rgb(183, 148, 244),
                good: Color::Rgb(143, 210, 163),
                warning: Color::Rgb(246, 199, 111),
                critical: Color::Rgb(243, 130, 148),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Theme;

    #[test]
    fn built_in_themes_have_stable_labels_and_wrap_when_cycled() {
        assert_eq!(Theme::ALL.len(), 10);
        assert_eq!(Theme::Default.label(), "Default");
        assert_eq!(Theme::Nord.label(), "Nord");
        assert_eq!(Theme::Dusk.next(), Theme::Default);
        assert_eq!(Theme::Default.previous(), Theme::Dusk);
    }

    #[test]
    fn original_themes_keep_text_and_selection_distinct_from_the_background() {
        for theme in [Theme::Ocean, Theme::Evergreen, Theme::Dusk] {
            let palette = theme.palette();
            assert_ne!(palette.foreground, palette.background);
            assert_ne!(palette.selection_foreground, palette.selection_background);
        }
    }
}
