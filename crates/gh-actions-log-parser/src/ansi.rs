//! ANSI escape sequence parsing using ansi-parser crate

use crate::types::{AnsiStyle, Color, NamedColor, StyledSegment};
use ansi_parser::{AnsiParser, AnsiSequence, Output};

/// Parse a line of text with ANSI escape sequences and return styled segments
pub fn parse_ansi_line(text: &str) -> Vec<StyledSegment> {
    let mut segments = Vec::new();
    let mut current_style = AnsiStyle::default();
    let mut current_text = String::new();

    for output in text.ansi_parse() {
        match output {
            Output::TextBlock(text) => {
                // Accumulate text with current style
                current_text.push_str(text);
            }
            Output::Escape(sequence) => {
                // Flush accumulated text before applying new style
                if !current_text.is_empty() {
                    segments.push(StyledSegment::with_style(
                        std::mem::take(&mut current_text),
                        current_style.clone(),
                    ));
                }

                // Update style based on ANSI sequence
                apply_ansi_sequence(&mut current_style, &sequence);
            }
        }
    }

    // Push any remaining text
    if !current_text.is_empty() {
        segments.push(StyledSegment::with_style(current_text, current_style));
    }

    // If no segments were created, return the original text as unstyled
    if segments.is_empty() {
        segments.push(StyledSegment::new(text.to_string()));
    }

    segments
}

/// Apply an ANSI escape sequence to the current style
fn apply_ansi_sequence(style: &mut AnsiStyle, sequence: &AnsiSequence) {
    use ansi_parser::AnsiSequence::*;

    // Only SetGraphicsMode affects text styling
    if let SetGraphicsMode(modes) = sequence {
        for mode in modes {
            apply_graphics_mode(style, *mode);
        }
    }
    // Cursor and other sequences don't affect text styling
}

/// Apply a single SGR (Select Graphic Rendition) parameter
fn apply_graphics_mode(style: &mut AnsiStyle, mode: u8) {
    match mode {
        // Reset/Normal
        0 => {
            *style = AnsiStyle::default();
        }

        // Intensity
        1 => style.bold = true,
        2 => style.faint = true,
        22 => {
            style.bold = false;
            style.faint = false;
        }

        // Italic
        3 => style.italic = true,
        23 => style.italic = false,

        // Underline
        4 => style.underline = true,
        24 => style.underline = false,

        // Blink
        5 | 6 => style.blink = true,
        25 => style.blink = false,

        // Reverse video
        7 => style.reversed = true,
        27 => style.reversed = false,

        // Conceal/Hidden
        8 => style.hidden = true,
        28 => style.hidden = false,

        // Strikethrough - DISABLED to avoid conflicts with 256-color parsing
        // The ansi-parser crate doesn't properly handle multi-parameter sequences
        // like [38;5;9m, and treats the 9 as a separate code (strikethrough)
        // 9 => style.strikethrough = true,
        29 => style.strikethrough = false,

        // Foreground colors (30-37: standard, 90-97: bright)
        30 => style.fg_color = Some(Color::Named(NamedColor::Black)),
        31 => style.fg_color = Some(Color::Named(NamedColor::Red)),
        32 => style.fg_color = Some(Color::Named(NamedColor::Green)),
        33 => style.fg_color = Some(Color::Named(NamedColor::Yellow)),
        34 => style.fg_color = Some(Color::Named(NamedColor::Blue)),
        35 => style.fg_color = Some(Color::Named(NamedColor::Magenta)),
        36 => style.fg_color = Some(Color::Named(NamedColor::Cyan)),
        37 => style.fg_color = Some(Color::Named(NamedColor::White)),
        39 => style.fg_color = None, // Default foreground

        90 => style.fg_color = Some(Color::Named(NamedColor::BrightBlack)),
        91 => style.fg_color = Some(Color::Named(NamedColor::BrightRed)),
        92 => style.fg_color = Some(Color::Named(NamedColor::BrightGreen)),
        93 => style.fg_color = Some(Color::Named(NamedColor::BrightYellow)),
        94 => style.fg_color = Some(Color::Named(NamedColor::BrightBlue)),
        95 => style.fg_color = Some(Color::Named(NamedColor::BrightMagenta)),
        96 => style.fg_color = Some(Color::Named(NamedColor::BrightCyan)),
        97 => style.fg_color = Some(Color::Named(NamedColor::BrightWhite)),

        // Background colors (40-47: standard, 100-107: bright)
        40 => style.bg_color = Some(Color::Named(NamedColor::Black)),
        41 => style.bg_color = Some(Color::Named(NamedColor::Red)),
        42 => style.bg_color = Some(Color::Named(NamedColor::Green)),
        43 => style.bg_color = Some(Color::Named(NamedColor::Yellow)),
        44 => style.bg_color = Some(Color::Named(NamedColor::Blue)),
        45 => style.bg_color = Some(Color::Named(NamedColor::Magenta)),
        46 => style.bg_color = Some(Color::Named(NamedColor::Cyan)),
        47 => style.bg_color = Some(Color::Named(NamedColor::White)),
        49 => style.bg_color = None, // Default background

        100 => style.bg_color = Some(Color::Named(NamedColor::BrightBlack)),
        101 => style.bg_color = Some(Color::Named(NamedColor::BrightRed)),
        102 => style.bg_color = Some(Color::Named(NamedColor::BrightGreen)),
        103 => style.bg_color = Some(Color::Named(NamedColor::BrightYellow)),
        104 => style.bg_color = Some(Color::Named(NamedColor::BrightBlue)),
        105 => style.bg_color = Some(Color::Named(NamedColor::BrightMagenta)),
        106 => style.bg_color = Some(Color::Named(NamedColor::BrightCyan)),
        107 => style.bg_color = Some(Color::Named(NamedColor::BrightWhite)),

        // Note: 256-color (38;5;n, 48;5;n) and RGB (38;2;r;g;b, 48;2;r;g;b)
        // sequences require parsing multiple parameters, which ansi-parser
        // currently doesn't expose in a way we can easily use.
        // These would need to be handled by looking at the raw sequence.

        // Ignore unknown codes
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_text() {
        let segments = parse_ansi_line("plain text");
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text, "plain text");
        assert!(segments[0].style.fg_color.is_none());
    }

    #[test]
    fn test_red_text() {
        let segments = parse_ansi_line("\x1b[31mred text\x1b[0m");
        // After reset, we may get 1 segment if the final reset doesn't create empty text
        assert!(!segments.is_empty());
        assert_eq!(segments[0].text, "red text");
        assert!(matches!(
            segments[0].style.fg_color,
            Some(Color::Named(NamedColor::Red))
        ));
    }

    #[test]
    fn test_bold_text() {
        let segments = parse_ansi_line("\x1b[1mbold\x1b[0m");
        // After reset, we may get 1 segment if the final reset doesn't create empty text
        assert!(!segments.is_empty());
        assert_eq!(segments[0].text, "bold");
        assert!(segments[0].style.bold);
    }

    #[test]
    fn test_multiple_styles() {
        let segments = parse_ansi_line("\x1b[1;31mbold red\x1b[0m normal");
        // Should have at least 2 segments: styled text and normal text
        assert!(segments.len() >= 2);
        assert_eq!(segments[0].text, "bold red");
        assert!(segments[0].style.bold);
        assert!(matches!(
            segments[0].style.fg_color,
            Some(Color::Named(NamedColor::Red))
        ));
        // Find the "normal" text segment
        let normal_segment = segments
            .iter()
            .find(|s| s.text.contains("normal"))
            .expect("Should have normal text");
        assert_eq!(normal_segment.text.trim(), "normal");
    }
}
