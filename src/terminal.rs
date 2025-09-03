use std::cmp;
use crate::terminal_parser::{TerminalParser, TerminalAction};
use tracing::debug;

#[derive(Debug, Clone)]
pub struct TerminalCell {
    pub character: char,
    pub foreground: [f32; 4],
    pub background: [f32; 4],
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub dim: bool,
    pub reverse: bool,
    pub blink: bool,
    pub wide: bool,
    pub dirty: bool,
}

impl Default for TerminalCell {
    fn default() -> Self {
        Self {
            character: ' ',
            foreground: [1.0, 1.0, 1.0, 1.0], // White
            background: [0.0, 0.0, 0.0, 1.0], // Black
            bold: false,
            italic: false,
            underline: false,
            strikethrough: false,
            dim: false,
            reverse: false,
            blink: false,
            wide: false,
            dirty: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TerminalState {
    // Grid
    pub width: u32,
    pub height: u32,
    pub cells: Vec<TerminalCell>,
    
    // Cursor
    pub cursor_x: u32,
    pub cursor_y: u32,
    pub cursor_visible: bool,
    
    // Current text attributes
    pub current_fg: [f32; 4],
    pub current_bg: [f32; 4],
    pub current_bold: bool,
    pub current_italic: bool,
    pub current_underline: bool,
    pub current_reverse: bool,
    
    // Terminal modes
    pub wrap_mode: bool,
    pub application_mode: bool,
    
    // Scrolling
    pub scroll_top: u32,
    pub scroll_bottom: u32,
    
    // Parser
    parser: TerminalParser,
}

impl TerminalState {
    pub fn new(width: u32, height: u32) -> Self {
        let cell_count = (width * height) as usize;
        let cells = vec![TerminalCell::default(); cell_count];
        
        Self {
            width,
            height,
            cells,
            cursor_x: 0,
            cursor_y: 0,
            cursor_visible: true,
            current_fg: [1.0, 1.0, 1.0, 1.0], // White
            current_bg: [0.0, 0.0, 0.0, 1.0], // Black
            current_bold: false,
            current_italic: false,
            current_underline: false,
            current_reverse: false,
            wrap_mode: true,
            application_mode: false,
            scroll_top: 0,
            scroll_bottom: height.saturating_sub(1),
            parser: TerminalParser::new(),
        }
    }
    
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == self.width && height == self.height {
            return;
        }
        
        let old_cells = std::mem::take(&mut self.cells);
        let old_width = self.width;
        let old_height = self.height;
        
        self.width = width;
        self.height = height;
        self.cells = vec![TerminalCell::default(); (width * height) as usize];
        
        // Copy old content to new grid
        let copy_width = cmp::min(old_width, width);
        let copy_height = cmp::min(old_height, height);
        
        for y in 0..copy_height {
            for x in 0..copy_width {
                let old_index = (y * old_width + x) as usize;
                let new_index = (y * width + x) as usize;
                
                if old_index < old_cells.len() && new_index < self.cells.len() {
                    self.cells[new_index] = old_cells[old_index].clone();
                }
            }
        }
        
        // Adjust cursor position
        self.cursor_x = cmp::min(self.cursor_x, width.saturating_sub(1));
        self.cursor_y = cmp::min(self.cursor_y, height.saturating_sub(1));
        
        // Adjust scroll region
        self.scroll_bottom = height.saturating_sub(1);
    }
    
    pub fn feed_bytes(&mut self, data: &[u8]) {
        let actions = self.parser.feed(data);
        
        for action in actions {
            self.execute_action(action);
        }
    }
    
    fn execute_action(&mut self, action: TerminalAction) {
        match action {
            TerminalAction::PrintChar(ch) => {
                self.print_char(ch);
            }
            TerminalAction::MoveCursor(row, col) => {
                self.cursor_y = cmp::min(row, self.height.saturating_sub(1));
                self.cursor_x = cmp::min(col, self.width.saturating_sub(1));
            }
            TerminalAction::MoveCursorUp(n) => {
                self.cursor_y = self.cursor_y.saturating_sub(n);
            }
            TerminalAction::MoveCursorDown(n) => {
                self.cursor_y = cmp::min(self.cursor_y + n, self.height.saturating_sub(1));
            }
            TerminalAction::MoveCursorLeft(n) => {
                self.cursor_x = self.cursor_x.saturating_sub(n);
            }
            TerminalAction::MoveCursorRight(n) => {
                self.cursor_x = cmp::min(self.cursor_x + n, self.width.saturating_sub(1));
            }
            TerminalAction::MoveCursorToColumn(col) => {
                self.cursor_x = cmp::min(col, self.width.saturating_sub(1));
            }
            TerminalAction::MoveCursorHome => {
                self.cursor_x = 0;
                self.cursor_y = 0;
            }
            TerminalAction::ClearLine => {
                self.clear_line(self.cursor_y);
            }
            TerminalAction::ClearLineFromCursor => {
                self.clear_line_from_cursor();
            }
            TerminalAction::ClearLineToCursor => {
                self.clear_line_to_cursor();
            }
            TerminalAction::ClearScreen => {
                self.clear_screen();
            }
            TerminalAction::ClearScreenFromCursor => {
                self.clear_screen_from_cursor();
            }
            TerminalAction::ClearScreenToCursor => {
                self.clear_screen_to_cursor();
            }
            TerminalAction::DeleteChar(n) => {
                self.delete_chars(n);
            }
            TerminalAction::InsertChar(n) => {
                self.insert_chars(n);
            }
            TerminalAction::SetForeground(color) => {
                self.current_fg = color.to_rgba();
            }
            TerminalAction::SetBackground(color) => {
                self.current_bg = color.to_rgba();
            }
            TerminalAction::SetBold(bold) => {
                self.current_bold = bold;
            }
            TerminalAction::SetItalic(italic) => {
                self.current_italic = italic;
            }
            TerminalAction::SetUnderline(underline) => {
                self.current_underline = underline;
            }
            TerminalAction::SetReverse(reverse) => {
                self.current_reverse = reverse;
            }
            TerminalAction::ResetAttributes => {
                self.current_fg = [1.0, 1.0, 1.0, 1.0]; // White
                self.current_bg = [0.0, 0.0, 0.0, 1.0]; // Black
                self.current_bold = false;
                self.current_italic = false;
                self.current_underline = false;
                self.current_reverse = false;
            }
            TerminalAction::ScrollUp(n) => {
                self.scroll_up(n);
            }
            TerminalAction::ScrollDown(n) => {
                self.scroll_down(n);
            }
            TerminalAction::Newline => {
                self.newline();
            }
            TerminalAction::CarriageReturn => {
                self.cursor_x = 0;
            }
            TerminalAction::Tab => {
                // Move to next tab stop (every 8 columns)
                let next_tab = ((self.cursor_x / 8) + 1) * 8;
                self.cursor_x = cmp::min(next_tab, self.width.saturating_sub(1));
            }
            TerminalAction::Bell => {
                // Visual bell - could flash the screen
                debug!("Bell");
            }
            TerminalAction::Backspace => {
                if self.cursor_x > 0 {
                    self.cursor_x -= 1;
                }
            }
            TerminalAction::ShowCursor => {
                self.cursor_visible = true;
            }
            TerminalAction::HideCursor => {
                self.cursor_visible = false;
            }
            TerminalAction::SetApplicationMode(enabled) => {
                self.application_mode = enabled;
            }
            TerminalAction::SetWrapMode(enabled) => {
                self.wrap_mode = enabled;
            }
        }
    }
    
    fn print_char(&mut self, ch: char) {
        if self.cursor_x >= self.width {
            if self.wrap_mode {
                self.cursor_x = 0;
                self.cursor_y += 1;
                if self.cursor_y >= self.height {
                    self.scroll_up(1);
                    self.cursor_y = self.height.saturating_sub(1);
                }
            } else {
                return; // Don't print if wrapping is disabled
            }
        }
        
        let index = (self.cursor_y * self.width + self.cursor_x) as usize;
        if index < self.cells.len() {
            let cell = &mut self.cells[index];
            cell.character = ch;
            cell.foreground = if self.current_reverse { self.current_bg } else { self.current_fg };
            cell.background = if self.current_reverse { self.current_fg } else { self.current_bg };
            cell.bold = self.current_bold;
            cell.italic = self.current_italic;
            cell.underline = self.current_underline;
            cell.reverse = self.current_reverse;
            cell.dirty = true;
        }
        
        self.cursor_x += 1;
    }
    
    fn newline(&mut self) {
        self.cursor_x = 0;
        self.cursor_y += 1;
        
        if self.cursor_y >= self.height {
            self.scroll_up(1);
            self.cursor_y = self.height.saturating_sub(1);
        }
    }
    
    fn clear_line(&mut self, y: u32) {
        if y >= self.height {
            return;
        }
        
        let start = (y * self.width) as usize;
        let end = start + self.width as usize;
        
        for i in start..end {
            if i < self.cells.len() {
                self.cells[i] = TerminalCell {
                    background: self.current_bg,
                    dirty: true,
                    ..Default::default()
                };
            }
        }
    }
    
    fn clear_line_from_cursor(&mut self) {
        let y = self.cursor_y;
        if y >= self.height {
            return;
        }
        
        let start = (y * self.width + self.cursor_x) as usize;
        let end = ((y + 1) * self.width) as usize;
        
        for i in start..end {
            if i < self.cells.len() {
                self.cells[i] = TerminalCell {
                    background: self.current_bg,
                    dirty: true,
                    ..Default::default()
                };
            }
        }
    }
    
    fn clear_line_to_cursor(&mut self) {
        let y = self.cursor_y;
        if y >= self.height {
            return;
        }
        
        let start = (y * self.width) as usize;
        let end = (y * self.width + self.cursor_x + 1) as usize;
        
        for i in start..end {
            if i < self.cells.len() {
                self.cells[i] = TerminalCell {
                    background: self.current_bg,
                    dirty: true,
                    ..Default::default()
                };
            }
        }
    }
    
    fn clear_screen(&mut self) {
        for cell in &mut self.cells {
            *cell = TerminalCell {
                background: self.current_bg,
                dirty: true,
                ..Default::default()
            };
        }
    }
    
    fn clear_screen_from_cursor(&mut self) {
        let start = (self.cursor_y * self.width + self.cursor_x) as usize;
        
        for i in start..self.cells.len() {
            self.cells[i] = TerminalCell {
                background: self.current_bg,
                dirty: true,
                ..Default::default()
            };
        }
    }
    
    fn clear_screen_to_cursor(&mut self) {
        let end = (self.cursor_y * self.width + self.cursor_x + 1) as usize;
        
        for i in 0..end.min(self.cells.len()) {
            self.cells[i] = TerminalCell {
                background: self.current_bg,
                dirty: true,
                ..Default::default()
            };
        }
    }
    
    fn delete_chars(&mut self, n: u32) {
        let y = self.cursor_y;
        if y >= self.height {
            return;
        }
        
        let _line_start = (y * self.width) as usize;
        let line_end = ((y + 1) * self.width) as usize;
        let delete_start = (y * self.width + self.cursor_x) as usize;
        
        // Shift characters left
        for i in 0..n {
            let src = delete_start + i as usize + n as usize;
            let dst = delete_start + i as usize;
            
            if src < line_end && dst < self.cells.len() && src < self.cells.len() {
                self.cells[dst] = self.cells[src].clone();
                self.cells[dst].dirty = true;
            }
        }
        
        // Clear the rightmost characters
        let clear_start = (line_end as u32).saturating_sub(n) as usize;
        for i in clear_start..line_end {
            if i < self.cells.len() {
                self.cells[i] = TerminalCell {
                    background: self.current_bg,
                    dirty: true,
                    ..Default::default()
                };
            }
        }
    }
    
    fn insert_chars(&mut self, n: u32) {
        let y = self.cursor_y;
        if y >= self.height {
            return;
        }
        
        let _line_start = (y * self.width) as usize;
        let line_end = ((y + 1) * self.width) as usize;
        let insert_start = (y * self.width + self.cursor_x) as usize;
        
        // Shift characters right
        for i in (0..(line_end - insert_start).saturating_sub(n as usize)).rev() {
            let src = insert_start + i;
            let dst = insert_start + i + n as usize;
            
            if src < self.cells.len() && dst < line_end && dst < self.cells.len() {
                self.cells[dst] = self.cells[src].clone();
                self.cells[dst].dirty = true;
            }
        }
        
        // Clear the inserted space
        for i in 0..n {
            let index = insert_start + i as usize;
            if index < self.cells.len() {
                self.cells[index] = TerminalCell {
                    background: self.current_bg,
                    dirty: true,
                    ..Default::default()
                };
            }
        }
    }
    
    fn scroll_up(&mut self, n: u32) {
        let scroll_lines = n.min(self.height);
        
        // Move lines up
        for dest_y in 0..self.height.saturating_sub(scroll_lines) {
            let src_y = dest_y + scroll_lines;
            
            let dest_start = (dest_y * self.width) as usize;
            let src_start = (src_y * self.width) as usize;
            
            for x in 0..self.width {
                let dest_index = dest_start + x as usize;
                let src_index = src_start + x as usize;
                
                if dest_index < self.cells.len() && src_index < self.cells.len() {
                    self.cells[dest_index] = self.cells[src_index].clone();
                    self.cells[dest_index].dirty = true;
                }
            }
        }
        
        // Clear the bottom lines
        let clear_start_y = self.height.saturating_sub(scroll_lines);
        for y in clear_start_y..self.height {
            self.clear_line(y);
        }
    }
    
    fn scroll_down(&mut self, n: u32) {
        let scroll_lines = n.min(self.height);
        
        // Move lines down
        for dest_y in (scroll_lines..self.height).rev() {
            let src_y = dest_y - scroll_lines;
            
            let dest_start = (dest_y * self.width) as usize;
            let src_start = (src_y * self.width) as usize;
            
            for x in 0..self.width {
                let dest_index = dest_start + x as usize;
                let src_index = src_start + x as usize;
                
                if dest_index < self.cells.len() && src_index < self.cells.len() {
                    self.cells[dest_index] = self.cells[src_index].clone();
                    self.cells[dest_index].dirty = true;
                }
            }
        }
        
        // Clear the top lines
        for y in 0..scroll_lines {
            self.clear_line(y);
        }
    }
    
    pub fn get_cell(&self, x: u32, y: u32) -> Option<&TerminalCell> {
        if x < self.width && y < self.height {
            let index = (y * self.width + x) as usize;
            self.cells.get(index)
        } else {
            None
        }
    }
    
    pub fn set_cell(&mut self, x: u32, y: u32, cell: TerminalCell) {
        if x < self.width && y < self.height {
            let index = (y * self.width + x) as usize;
            if index < self.cells.len() {
                self.cells[index] = cell;
            }
        }
    }
    
    pub fn clear_dirty_flags(&mut self) {
        for cell in &mut self.cells {
            cell.dirty = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terminal_creation() {
        let terminal = TerminalState::new(80, 24);
        assert_eq!(terminal.width, 80);
        assert_eq!(terminal.height, 24);
        assert_eq!(terminal.cells.len(), 80 * 24);
        assert_eq!(terminal.cursor_x, 0);
        assert_eq!(terminal.cursor_y, 0);
    }
    
    #[test]
    fn test_print_char() {
        let mut terminal = TerminalState::new(80, 24);
        terminal.feed_bytes(b"Hello");
        
        assert_eq!(terminal.cursor_x, 5);
        assert_eq!(terminal.cursor_y, 0);
        
        if let Some(cell) = terminal.get_cell(0, 0) {
            assert_eq!(cell.character, 'H');
        }
        if let Some(cell) = terminal.get_cell(4, 0) {
            assert_eq!(cell.character, 'o');
        }
    }
    
    #[test]
    fn test_newline() {
        let mut terminal = TerminalState::new(80, 24);
        terminal.feed_bytes(b"Hello\nWorld");
        
        assert_eq!(terminal.cursor_x, 5);
        assert_eq!(terminal.cursor_y, 1);
        
        if let Some(cell) = terminal.get_cell(0, 1) {
            assert_eq!(cell.character, 'W');
        }
    }
    
    #[test]
    fn test_cursor_movement() {
        let mut terminal = TerminalState::new(80, 24);
        terminal.feed_bytes(b"\x1b[10;20H");
        
        assert_eq!(terminal.cursor_x, 19); // 0-based
        assert_eq!(terminal.cursor_y, 9);  // 0-based
    }
    
    #[test]
    fn test_clear_screen() {
        let mut terminal = TerminalState::new(80, 24);
        terminal.feed_bytes(b"Hello");
        terminal.feed_bytes(b"\x1b[2J");
        
        if let Some(cell) = terminal.get_cell(0, 0) {
            assert_eq!(cell.character, ' ');
        }
    }
}