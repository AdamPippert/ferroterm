use std::collections::VecDeque;
use thiserror::Error;
use tracing::{debug, warn};

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("Invalid escape sequence: {0}")]
    InvalidSequence(String),
    #[error("Incomplete escape sequence")]
    IncompleteSequence,
    #[error("Buffer overflow")]
    BufferOverflow,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TerminalAction {
    // Character output
    PrintChar(char),
    
    // Cursor movement
    MoveCursor(u32, u32), // row, col (0-based)
    MoveCursorUp(u32),
    MoveCursorDown(u32),
    MoveCursorLeft(u32),
    MoveCursorRight(u32),
    MoveCursorToColumn(u32),
    MoveCursorHome,
    
    // Text modification
    ClearLine,
    ClearLineFromCursor,
    ClearLineToCursor,
    ClearScreen,
    ClearScreenFromCursor,
    ClearScreenToCursor,
    DeleteChar(u32),
    InsertChar(u32),
    
    // Text attributes
    SetForeground(Color),
    SetBackground(Color),
    SetBold(bool),
    SetItalic(bool),
    SetUnderline(bool),
    SetReverse(bool),
    ResetAttributes,
    
    // Scrolling
    ScrollUp(u32),
    ScrollDown(u32),
    
    // Special
    Newline,
    CarriageReturn,
    Tab,
    Bell,
    Backspace,
    
    // Cursor visibility
    ShowCursor,
    HideCursor,
    
    // Terminal modes
    SetApplicationMode(bool),
    SetWrapMode(bool),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Color {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    BrightBlack,
    BrightRed,
    BrightGreen,
    BrightYellow,
    BrightBlue,
    BrightMagenta,
    BrightCyan,
    BrightWhite,
    Color256(u8),
    TrueColor(u8, u8, u8),
    Default,
}

impl Color {
    pub fn to_rgba(&self) -> [f32; 4] {
        match self {
            Color::Black => [0.0, 0.0, 0.0, 1.0],
            Color::Red => [1.0, 0.0, 0.0, 1.0],
            Color::Green => [0.0, 1.0, 0.0, 1.0],
            Color::Yellow => [1.0, 1.0, 0.0, 1.0],
            Color::Blue => [0.0, 0.0, 1.0, 1.0],
            Color::Magenta => [1.0, 0.0, 1.0, 1.0],
            Color::Cyan => [0.0, 1.0, 1.0, 1.0],
            Color::White => [1.0, 1.0, 1.0, 1.0],
            Color::BrightBlack => [0.5, 0.5, 0.5, 1.0],
            Color::BrightRed => [1.0, 0.5, 0.5, 1.0],
            Color::BrightGreen => [0.5, 1.0, 0.5, 1.0],
            Color::BrightYellow => [1.0, 1.0, 0.5, 1.0],
            Color::BrightBlue => [0.5, 0.5, 1.0, 1.0],
            Color::BrightMagenta => [1.0, 0.5, 1.0, 1.0],
            Color::BrightCyan => [0.5, 1.0, 1.0, 1.0],
            Color::BrightWhite => [1.0, 1.0, 1.0, 1.0],
            Color::TrueColor(r, g, b) => [*r as f32 / 255.0, *g as f32 / 255.0, *b as f32 / 255.0, 1.0],
            Color::Color256(n) => {
                // Convert 256-color palette to RGB (simplified)
                let n = *n as f32;
                if n < 16.0 {
                    // Standard colors
                    match n as u8 {
                        0..=7 => Color::Black.to_rgba(),
                        8..=15 => Color::BrightBlack.to_rgba(),
                        _ => [1.0, 1.0, 1.0, 1.0],
                    }
                } else if n < 232.0 {
                    // 216 color cube
                    let n = n - 16.0;
                    let r = (n / 36.0) as u8;
                    let g = ((n % 36.0) / 6.0) as u8;
                    let b = (n % 6.0) as u8;
                    [r as f32 * 0.2, g as f32 * 0.2, b as f32 * 0.2, 1.0]
                } else {
                    // Grayscale
                    let gray = (n - 232.0) / 24.0;
                    [gray, gray, gray, 1.0]
                }
            }
            Color::Default => [1.0, 1.0, 1.0, 1.0], // White
        }
    }
}

#[derive(Debug, Clone)]
pub struct TerminalParser {
    buffer: VecDeque<u8>,
    state: ParserState,
    params: Vec<u32>,
    current_param: String,
}

#[derive(Debug, Clone, PartialEq)]
enum ParserState {
    Normal,
    Escape,
    CSI,
    OSC,
}

impl Default for TerminalParser {
    fn default() -> Self {
        Self::new()
    }
}

impl TerminalParser {
    pub fn new() -> Self {
        Self {
            buffer: VecDeque::new(),
            state: ParserState::Normal,
            params: Vec::new(),
            current_param: String::new(),
        }
    }

    pub fn feed(&mut self, data: &[u8]) -> Vec<TerminalAction> {
        let mut actions = Vec::new();
        
        for &byte in data {
            self.buffer.push_back(byte);
        }

        while let Some(byte) = self.buffer.pop_front() {
            match self.parse_byte(byte) {
                Ok(Some(action)) => actions.push(action),
                Ok(None) => {}, // Continue parsing
                Err(e) => {
                    warn!("Parse error: {}", e);
                    // Reset parser state on error
                    self.reset_state();
                }
            }
        }

        actions
    }

    fn parse_byte(&mut self, byte: u8) -> Result<Option<TerminalAction>, ParseError> {
        match self.state {
            ParserState::Normal => self.parse_normal(byte),
            ParserState::Escape => self.parse_escape(byte),
            ParserState::CSI => self.parse_csi(byte),
            ParserState::OSC => self.parse_osc(byte),
        }
    }

    fn parse_normal(&mut self, byte: u8) -> Result<Option<TerminalAction>, ParseError> {
        match byte {
            0x1B => { // ESC
                self.state = ParserState::Escape;
                Ok(None)
            }
            0x08 => Ok(Some(TerminalAction::Backspace)),
            0x09 => Ok(Some(TerminalAction::Tab)),
            0x0A => Ok(Some(TerminalAction::Newline)),
            0x0D => Ok(Some(TerminalAction::CarriageReturn)),
            0x07 => Ok(Some(TerminalAction::Bell)),
            0x20..=0x7E => {
                // Printable ASCII
                Ok(Some(TerminalAction::PrintChar(byte as char)))
            }
            0x80..=0xFF => {
                // Handle UTF-8 sequences (simplified)
                Ok(Some(TerminalAction::PrintChar(byte as char)))
            }
            _ => {
                // Control characters - ignore for now
                debug!("Ignoring control character: 0x{:02X}", byte);
                Ok(None)
            }
        }
    }

    fn parse_escape(&mut self, byte: u8) -> Result<Option<TerminalAction>, ParseError> {
        match byte {
            b'[' => {
                self.state = ParserState::CSI;
                self.params.clear();
                self.current_param.clear();
                Ok(None)
            }
            b']' => {
                self.state = ParserState::OSC;
                Ok(None)
            }
            b'M' => {
                self.state = ParserState::Normal;
                Ok(Some(TerminalAction::ScrollUp(1)))
            }
            b'D' => {
                self.state = ParserState::Normal;
                Ok(Some(TerminalAction::ScrollDown(1)))
            }
            b'H' => {
                self.state = ParserState::Normal;
                // Set tab stop - not implemented
                Ok(None)
            }
            b'=' => {
                self.state = ParserState::Normal;
                Ok(Some(TerminalAction::SetApplicationMode(true)))
            }
            b'>' => {
                self.state = ParserState::Normal;
                Ok(Some(TerminalAction::SetApplicationMode(false)))
            }
            _ => {
                self.state = ParserState::Normal;
                Err(ParseError::InvalidSequence(format!("ESC {}", byte as char)))
            }
        }
    }

    fn parse_csi(&mut self, byte: u8) -> Result<Option<TerminalAction>, ParseError> {
        match byte {
            b'0'..=b'9' => {
                self.current_param.push(byte as char);
                Ok(None)
            }
            b';' => {
                self.push_param();
                Ok(None)
            }
            // Cursor movement
            b'A' => {
                self.push_param();
                let n = self.params.get(0).copied().unwrap_or(1);
                self.reset_state();
                Ok(Some(TerminalAction::MoveCursorUp(n)))
            }
            b'B' => {
                self.push_param();
                let n = self.params.get(0).copied().unwrap_or(1);
                self.reset_state();
                Ok(Some(TerminalAction::MoveCursorDown(n)))
            }
            b'C' => {
                self.push_param();
                let n = self.params.get(0).copied().unwrap_or(1);
                self.reset_state();
                Ok(Some(TerminalAction::MoveCursorRight(n)))
            }
            b'D' => {
                self.push_param();
                let n = self.params.get(0).copied().unwrap_or(1);
                self.reset_state();
                Ok(Some(TerminalAction::MoveCursorLeft(n)))
            }
            b'H' | b'f' => {
                self.push_param();
                let row = self.params.get(0).copied().unwrap_or(1).saturating_sub(1);
                let col = self.params.get(1).copied().unwrap_or(1).saturating_sub(1);
                self.reset_state();
                Ok(Some(TerminalAction::MoveCursor(row, col)))
            }
            b'G' => {
                self.push_param();
                let col = self.params.get(0).copied().unwrap_or(1).saturating_sub(1);
                self.reset_state();
                Ok(Some(TerminalAction::MoveCursorToColumn(col)))
            }
            // Clearing
            b'J' => {
                self.push_param();
                let mode = self.params.get(0).copied().unwrap_or(0);
                self.reset_state();
                match mode {
                    0 => Ok(Some(TerminalAction::ClearScreenFromCursor)),
                    1 => Ok(Some(TerminalAction::ClearScreenToCursor)),
                    2 => Ok(Some(TerminalAction::ClearScreen)),
                    _ => Ok(None),
                }
            }
            b'K' => {
                self.push_param();
                let mode = self.params.get(0).copied().unwrap_or(0);
                self.reset_state();
                match mode {
                    0 => Ok(Some(TerminalAction::ClearLineFromCursor)),
                    1 => Ok(Some(TerminalAction::ClearLineToCursor)),
                    2 => Ok(Some(TerminalAction::ClearLine)),
                    _ => Ok(None),
                }
            }
            // Character manipulation
            b'P' => {
                self.push_param();
                let n = self.params.get(0).copied().unwrap_or(1);
                self.reset_state();
                Ok(Some(TerminalAction::DeleteChar(n)))
            }
            b'@' => {
                self.push_param();
                let n = self.params.get(0).copied().unwrap_or(1);
                self.reset_state();
                Ok(Some(TerminalAction::InsertChar(n)))
            }
            // Text attributes (SGR)
            b'm' => {
                self.push_param();
                let actions = self.parse_sgr();
                self.reset_state();
                // Return the first action for now (simplified)
                Ok(actions.into_iter().next())
            }
            // Cursor visibility
            b'l' if self.params.get(0) == Some(&25) => {
                self.reset_state();
                Ok(Some(TerminalAction::HideCursor))
            }
            b'h' if self.params.get(0) == Some(&25) => {
                self.reset_state();
                Ok(Some(TerminalAction::ShowCursor))
            }
            _ => {
                self.reset_state();
                Err(ParseError::InvalidSequence(format!("CSI {}", byte as char)))
            }
        }
    }

    fn parse_osc(&mut self, byte: u8) -> Result<Option<TerminalAction>, ParseError> {
        // OSC sequences (Operating System Commands) - simplified handling
        match byte {
            0x07 | 0x1B => { // BEL or ESC (terminator)
                self.state = ParserState::Normal;
                // For now, ignore OSC sequences
                Ok(None)
            }
            _ => {
                // Continue collecting OSC data
                Ok(None)
            }
        }
    }

    fn push_param(&mut self) {
        if !self.current_param.is_empty() {
            if let Ok(param) = self.current_param.parse::<u32>() {
                self.params.push(param);
            }
            self.current_param.clear();
        }
    }

    fn reset_state(&mut self) {
        self.state = ParserState::Normal;
        self.params.clear();
        self.current_param.clear();
    }

    fn parse_sgr(&self) -> Vec<TerminalAction> {
        let mut actions = Vec::new();
        
        if self.params.is_empty() {
            return vec![TerminalAction::ResetAttributes];
        }

        let mut i = 0;
        while i < self.params.len() {
            match self.params[i] {
                0 => actions.push(TerminalAction::ResetAttributes),
                1 => actions.push(TerminalAction::SetBold(true)),
                3 => actions.push(TerminalAction::SetItalic(true)),
                4 => actions.push(TerminalAction::SetUnderline(true)),
                7 => actions.push(TerminalAction::SetReverse(true)),
                22 => actions.push(TerminalAction::SetBold(false)),
                23 => actions.push(TerminalAction::SetItalic(false)),
                24 => actions.push(TerminalAction::SetUnderline(false)),
                27 => actions.push(TerminalAction::SetReverse(false)),
                30 => actions.push(TerminalAction::SetForeground(Color::Black)),
                31 => actions.push(TerminalAction::SetForeground(Color::Red)),
                32 => actions.push(TerminalAction::SetForeground(Color::Green)),
                33 => actions.push(TerminalAction::SetForeground(Color::Yellow)),
                34 => actions.push(TerminalAction::SetForeground(Color::Blue)),
                35 => actions.push(TerminalAction::SetForeground(Color::Magenta)),
                36 => actions.push(TerminalAction::SetForeground(Color::Cyan)),
                37 => actions.push(TerminalAction::SetForeground(Color::White)),
                39 => actions.push(TerminalAction::SetForeground(Color::Default)),
                40 => actions.push(TerminalAction::SetBackground(Color::Black)),
                41 => actions.push(TerminalAction::SetBackground(Color::Red)),
                42 => actions.push(TerminalAction::SetBackground(Color::Green)),
                43 => actions.push(TerminalAction::SetBackground(Color::Yellow)),
                44 => actions.push(TerminalAction::SetBackground(Color::Blue)),
                45 => actions.push(TerminalAction::SetBackground(Color::Magenta)),
                46 => actions.push(TerminalAction::SetBackground(Color::Cyan)),
                47 => actions.push(TerminalAction::SetBackground(Color::White)),
                49 => actions.push(TerminalAction::SetBackground(Color::Default)),
                90..=97 => {
                    let color = match self.params[i] {
                        90 => Color::BrightBlack,
                        91 => Color::BrightRed,
                        92 => Color::BrightGreen,
                        93 => Color::BrightYellow,
                        94 => Color::BrightBlue,
                        95 => Color::BrightMagenta,
                        96 => Color::BrightCyan,
                        97 => Color::BrightWhite,
                        _ => Color::Default,
                    };
                    actions.push(TerminalAction::SetForeground(color));
                }
                100..=107 => {
                    let color = match self.params[i] {
                        100 => Color::BrightBlack,
                        101 => Color::BrightRed,
                        102 => Color::BrightGreen,
                        103 => Color::BrightYellow,
                        104 => Color::BrightBlue,
                        105 => Color::BrightMagenta,
                        106 => Color::BrightCyan,
                        107 => Color::BrightWhite,
                        _ => Color::Default,
                    };
                    actions.push(TerminalAction::SetBackground(color));
                }
                38 => {
                    // Foreground color (256-color or RGB)
                    if i + 1 < self.params.len() {
                        match self.params[i + 1] {
                            5 => {
                                // 256-color mode
                                if i + 2 < self.params.len() {
                                    let color = Color::Color256(self.params[i + 2] as u8);
                                    actions.push(TerminalAction::SetForeground(color));
                                    i += 2;
                                }
                            }
                            2 => {
                                // RGB mode
                                if i + 4 < self.params.len() {
                                    let r = self.params[i + 2] as u8;
                                    let g = self.params[i + 3] as u8;
                                    let b = self.params[i + 4] as u8;
                                    let color = Color::TrueColor(r, g, b);
                                    actions.push(TerminalAction::SetForeground(color));
                                    i += 4;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                48 => {
                    // Background color (256-color or RGB)
                    if i + 1 < self.params.len() {
                        match self.params[i + 1] {
                            5 => {
                                // 256-color mode
                                if i + 2 < self.params.len() {
                                    let color = Color::Color256(self.params[i + 2] as u8);
                                    actions.push(TerminalAction::SetBackground(color));
                                    i += 2;
                                }
                            }
                            2 => {
                                // RGB mode
                                if i + 4 < self.params.len() {
                                    let r = self.params[i + 2] as u8;
                                    let g = self.params[i + 3] as u8;
                                    let b = self.params[i + 4] as u8;
                                    let color = Color::TrueColor(r, g, b);
                                    actions.push(TerminalAction::SetBackground(color));
                                    i += 4;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {
                    debug!("Unknown SGR parameter: {}", self.params[i]);
                }
            }
            i += 1;
        }

        if actions.is_empty() {
            actions.push(TerminalAction::ResetAttributes);
        }

        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_parsing() {
        let mut parser = TerminalParser::new();
        let actions = parser.feed(b"Hello");
        
        assert_eq!(actions.len(), 5);
        assert_eq!(actions[0], TerminalAction::PrintChar('H'));
        assert_eq!(actions[1], TerminalAction::PrintChar('e'));
        assert_eq!(actions[2], TerminalAction::PrintChar('l'));
        assert_eq!(actions[3], TerminalAction::PrintChar('l'));
        assert_eq!(actions[4], TerminalAction::PrintChar('o'));
    }

    #[test]
    fn test_cursor_movement() {
        let mut parser = TerminalParser::new();
        let actions = parser.feed(b"\x1b[2;3H");
        
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0], TerminalAction::MoveCursor(1, 2)); // 0-based
    }

    #[test]
    fn test_clear_screen() {
        let mut parser = TerminalParser::new();
        let actions = parser.feed(b"\x1b[2J");
        
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0], TerminalAction::ClearScreen);
    }

    #[test]
    fn test_colors() {
        let mut parser = TerminalParser::new();
        let actions = parser.feed(b"\x1b[31m");
        
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0], TerminalAction::SetForeground(Color::Red));
    }

    #[test]
    fn test_newline() {
        let mut parser = TerminalParser::new();
        let actions = parser.feed(b"\n");
        
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0], TerminalAction::Newline);
    }
}