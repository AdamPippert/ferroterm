use crate::renderer::{TerminalCell, TerminalGrid};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use pulldown_cmark::{Event, Parser, Tag, CowStr, HeadingLevel};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use syntect::highlighting::{HighlightState, Highlighter, Style, Theme, ThemeSet};
use syntect::parsing::{ParseState, SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;
use textwrap::{wrap, Options};
use thiserror::Error;
use tokio::sync::mpsc;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Error, Debug)]
pub enum MarkdownError {
    #[error("Parsing error: {0}")]
    Parse(String),
    #[error("Syntax highlighting error: {0}")]
    Highlighting(String),
    #[error("Rendering error: {0}")]
    Rendering(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Unicode error: {0}")]
    Unicode(String),
}

#[derive(Debug, Clone)]
pub struct TerminalColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl TerminalColor {
    pub fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub fn to_rgba_f32(&self) -> [f32; 4] {
        [
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
            self.a as f32 / 255.0,
        ]
    }

    pub fn from_syntect(color: syntect::highlighting::Color) -> Self {
        Self {
            r: color.r,
            g: color.g,
            b: color.b,
            a: color.a,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StyledText {
    pub text: String,
    pub color: TerminalColor,
    pub background: Option<TerminalColor>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
}

impl StyledText {
    pub fn new(text: String) -> Self {
        Self {
            text,
            color: TerminalColor::new(255, 255, 255), // Default white
            background: None,
            bold: false,
            italic: false,
            underline: false,
            strikethrough: false,
        }
    }

    pub fn with_color(mut self, color: TerminalColor) -> Self {
        self.color = color;
        self
    }

    pub fn with_background(mut self, color: TerminalColor) -> Self {
        self.background = Some(color);
        self
    }

    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    pub fn underline(mut self) -> Self {
        self.underline = true;
        self
    }

    pub fn strikethrough(mut self) -> Self {
        self.strikethrough = true;
        self
    }
}

#[derive(Debug, Clone)]
pub enum MarkdownElement {
    Text(StyledText),
    Header { level: u8, content: Vec<StyledText> },
    Paragraph(Vec<StyledText>),
    CodeBlock { language: Option<String>, content: String },
    InlineCode(String),
    List { ordered: bool, items: Vec<Vec<StyledText>> },
    Table { headers: Vec<String>, rows: Vec<Vec<String>> },
    Blockquote(Vec<MarkdownElement>),
    ThematicBreak,
    Link { url: String, text: Vec<StyledText> },
    Image { url: String, alt: String },
    LineBreak,
}

#[derive(Debug, Clone)]
pub struct RenderContext {
    pub terminal_width: usize,
    pub supports_truecolor: bool,
    pub supports_256color: bool,
    pub supports_unicode: bool,
    pub tab_width: usize,
    pub code_theme: String,
    pub wrap_code: bool,
    pub show_line_numbers: bool,
}

impl Default for RenderContext {
    fn default() -> Self {
        Self {
            terminal_width: 80,
            supports_truecolor: true,
            supports_256color: true,
            supports_unicode: true,
            tab_width: 4,
            code_theme: "base16-ocean.dark".to_string(),
            wrap_code: false,
            show_line_numbers: false,
        }
    }
}

pub struct MarkdownRenderer {
    syntax_set: SyntaxSet,
    theme_set: ThemeSet,
    current_theme: Arc<Theme>,
    context: RenderContext,
    streaming_buffer: String,
    partial_elements: VecDeque<MarkdownElement>,
    highlighter_cache: HashMap<String, (SyntaxReference, HighlightState, ParseState)>,
    performance_stats: PerformanceStats,
}

#[derive(Debug, Default)]
pub struct PerformanceStats {
    pub total_parse_time: Duration,
    pub total_render_time: Duration,
    pub total_bytes_processed: usize,
    pub render_calls: usize,
}

// Static syntax and theme sets for better performance
static SYNTAX_SET: Lazy<SyntaxSet> = Lazy::new(|| {
    let mut builder = SyntaxSet::load_defaults_newlines().into_builder();
    // Add custom syntax definitions if needed
    builder.build()
});

static THEME_SET: Lazy<ThemeSet> = Lazy::new(|| {
    ThemeSet::load_defaults()
});

impl MarkdownRenderer {
    pub fn new(context: RenderContext) -> Result<Self, MarkdownError> {
        let theme_name = &context.code_theme;
        let current_theme = Arc::new(
            THEME_SET
                .themes
                .get(theme_name)
                .or_else(|| THEME_SET.themes.get("base16-ocean.dark"))
                .or_else(|| THEME_SET.themes.values().next())
                .ok_or_else(|| MarkdownError::Highlighting("No themes available".to_string()))?
                .clone(),
        );

        Ok(Self {
            syntax_set: SYNTAX_SET.clone(),
            theme_set: THEME_SET.clone(),
            current_theme,
            context,
            streaming_buffer: String::new(),
            partial_elements: VecDeque::new(),
            highlighter_cache: HashMap::new(),
            performance_stats: PerformanceStats::default(),
        })
    }

    /// Parse markdown content into structured elements
    pub fn parse_markdown(&mut self, content: &str) -> Result<Vec<MarkdownElement>, MarkdownError> {
        let start_time = Instant::now();
        
        let parser = Parser::new(content);
        let mut elements = Vec::new();
        let mut current_text = Vec::new();
        let mut stack = Vec::new();
        let mut in_code_block = false;
        let mut code_language = None;
        let mut code_content = String::new();

        for event in parser {
            match event {
                Event::Start(tag) => {
                    stack.push((tag.clone(), current_text.clone()));
                    current_text.clear();
                    
                    match tag {
                        Tag::CodeBlock(kind) => {
                            in_code_block = true;
                            code_language = match kind {
                                pulldown_cmark::CodeBlockKind::Fenced(lang) => {
                                    if lang.is_empty() { None } else { Some(lang.to_string()) }
                                }
                                pulldown_cmark::CodeBlockKind::Indented => None,
                            };
                            code_content.clear();
                        }
                        _ => {}
                    }
                }
                Event::End(tag) => {
                    let (start_tag, mut parent_text) = stack.pop().unwrap_or((tag.clone(), Vec::new()));
                    
                    match tag {
                        Tag::Heading(level, _, _) => {
                            let heading_level = match level {
                                HeadingLevel::H1 => 1,
                                HeadingLevel::H2 => 2,
                                HeadingLevel::H3 => 3,
                                HeadingLevel::H4 => 4,
                                HeadingLevel::H5 => 5,
                                HeadingLevel::H6 => 6,
                            };
                            elements.push(MarkdownElement::Header {
                                level: heading_level,
                                content: current_text,
                            });
                        }
                        Tag::Paragraph => {
                            if !current_text.is_empty() {
                                elements.push(MarkdownElement::Paragraph(current_text));
                            }
                        }
                        Tag::CodeBlock(_) => {
                            in_code_block = false;
                            elements.push(MarkdownElement::CodeBlock {
                                language: code_language.take(),
                                content: code_content.clone(),
                            });
                            code_content.clear();
                        }
                        Tag::List(start) => {
                            // Handle list items (simplified for now)
                            let items = current_text.into_iter().map(|t| vec![t]).collect();
                            elements.push(MarkdownElement::List {
                                ordered: start.is_some(),
                                items,
                            });
                        }
                        Tag::BlockQuote => {
                            // Convert current text to paragraph elements
                            let quote_elements = vec![MarkdownElement::Paragraph(current_text)];
                            elements.push(MarkdownElement::Blockquote(quote_elements));
                        }
                        Tag::Link(_, url, _) => {
                            parent_text.push(StyledText::new("".to_string())); // Placeholder for link handling
                            current_text = parent_text;
                        }
                        _ => {
                            parent_text.extend(current_text);
                            current_text = parent_text;
                        }
                    }
                }
                Event::Text(text) => {
                    if in_code_block {
                        code_content.push_str(&text);
                    } else {
                        current_text.push(StyledText::new(text.to_string()));
                    }
                }
                Event::Code(text) => {
                    let mut code_text = StyledText::new(text.to_string());
                    code_text.background = Some(TerminalColor::new(40, 40, 40));
                    code_text.color = TerminalColor::new(200, 200, 200);
                    current_text.push(code_text);
                }
                Event::Html(html) => {
                    // For now, treat HTML as plain text
                    current_text.push(StyledText::new(html.to_string()));
                }
                Event::FootnoteReference(_) => {
                    // Skip footnotes for terminal rendering
                }
                Event::SoftBreak => {
                    current_text.push(StyledText::new(" ".to_string()));
                }
                Event::HardBreak => {
                    elements.push(MarkdownElement::LineBreak);
                }
                Event::Rule => {
                    elements.push(MarkdownElement::ThematicBreak);
                }
                Event::TaskListMarker(_) => {
                    // Handle task list markers if needed
                }
            }
        }

        // Handle any remaining text
        if !current_text.is_empty() {
            elements.push(MarkdownElement::Paragraph(current_text));
        }

        self.performance_stats.total_parse_time += start_time.elapsed();
        self.performance_stats.total_bytes_processed += content.len();

        Ok(elements)
    }

    /// Parse streaming markdown content (handles partial input)
    pub fn parse_streaming(&mut self, chunk: &str) -> Result<Vec<MarkdownElement>, MarkdownError> {
        self.streaming_buffer.push_str(chunk);
        
        // Try to parse complete elements from the buffer
        let mut elements = Vec::new();
        
        // Simple heuristic: try to parse when we have complete lines or blocks
        if self.streaming_buffer.contains("\n\n") || chunk.ends_with('\n') {
            // Attempt to parse what we have so far
            match self.parse_markdown(&self.streaming_buffer) {
                Ok(mut parsed) => {
                    // Store partial elements for potential reprocessing
                    elements.append(&mut parsed);
                    
                    // In a real implementation, we'd be smarter about what to keep in the buffer
                    // For now, clear if we successfully parsed
                    if !elements.is_empty() {
                        self.streaming_buffer.clear();
                    }
                }
                Err(_) => {
                    // Keep the buffer for next chunk if parsing failed
                }
            }
        }
        
        Ok(elements)
    }

    /// Highlight code using syntect
    pub fn highlight_code(&mut self, code: &str, language: Option<&str>) -> Result<Vec<StyledText>, MarkdownError> {
        let syntax = match language {
            Some(lang) => {
                self.syntax_set.find_syntax_by_token(lang)
                    .or_else(|| self.syntax_set.find_syntax_by_extension(lang))
                    .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text())
            }
            None => self.syntax_set.find_syntax_plain_text(),
        };

        let mut highlighter = Highlighter::new(&self.current_theme);
        let mut parse_state = ParseState::new(syntax);
        let mut highlighted_lines = Vec::new();

        for line in LinesWithEndings::from(code) {
            let ops = parse_state.parse_line(line, &self.syntax_set)
                .map_err(|e| MarkdownError::Highlighting(format!("Parse error: {:?}", e)))?;
            
            let escaped = highlighter.highlight_line(&ops, &self.syntax_set)
                .map_err(|e| MarkdownError::Highlighting(format!("Highlight error: {:?}", e)))?;
            
            let mut line_text = Vec::new();
            for (style, text) in escaped {
                let color = TerminalColor::from_syntect(style.foreground);
                let background = if style.background != self.current_theme.settings.background.unwrap_or_default() {
                    Some(TerminalColor::from_syntect(style.background))
                } else {
                    None
                };
                
                let mut styled = StyledText::new(text.to_string()).with_color(color);
                if let Some(bg) = background {
                    styled = styled.with_background(bg);
                }
                if style.font_style.contains(syntect::highlighting::FontStyle::BOLD) {
                    styled = styled.bold();
                }
                if style.font_style.contains(syntect::highlighting::FontStyle::ITALIC) {
                    styled = styled.italic();
                }
                if style.font_style.contains(syntect::highlighting::FontStyle::UNDERLINE) {
                    styled = styled.underline();
                }
                
                line_text.push(styled);
            }
            highlighted_lines.extend(line_text);
        }

        Ok(highlighted_lines)
    }

    /// Render markdown elements to terminal cells
    pub fn render_to_cells(&mut self, elements: &[MarkdownElement]) -> Result<Vec<TerminalCell>, MarkdownError> {
        let start_time = Instant::now();
        let mut cells = Vec::new();
        let mut current_row = 0;
        let mut current_col = 0;

        for element in elements {
            match element {
                MarkdownElement::Header { level, content } => {
                    // Add spacing before header
                    if current_row > 0 {
                        current_row += 1;
                        current_col = 0;
                    }

                    // Render header with appropriate styling
                    let header_color = match level {
                        1 => TerminalColor::new(255, 100, 100), // Red
                        2 => TerminalColor::new(100, 255, 100), // Green
                        3 => TerminalColor::new(100, 100, 255), // Blue
                        4 => TerminalColor::new(255, 255, 100), // Yellow
                        5 => TerminalColor::new(255, 100, 255), // Magenta
                        _ => TerminalColor::new(100, 255, 255), // Cyan
                    };

                    // Render header prefix
                    let prefix = "#".repeat(*level as usize) + " ";
                    for ch in prefix.chars() {
                        if current_col >= self.context.terminal_width {
                            current_row += 1;
                            current_col = 0;
                        }
                        cells.push(TerminalCell {
                            character: ch,
                            foreground: header_color.to_rgba_f32(),
                            background: [0.0, 0.0, 0.0, 1.0],
                            bold: true,
                            italic: false,
                            underline: *level <= 2, // Underline H1 and H2
                            strikethrough: false,
                            dim: false,
                            reverse: false,
                            blink: false,
                            wide: false,
                            double_height: false,
                            dirty: true,
                        });
                        current_col += 1;
                    }

                    // Render header content
                    for styled_text in content {
                        for ch in styled_text.text.chars() {
                            if current_col >= self.context.terminal_width {
                                current_row += 1;
                                current_col = 0;
                            }
                            cells.push(TerminalCell {
                                character: ch,
                                foreground: header_color.to_rgba_f32(),
                                background: styled_text.background
                                    .as_ref()
                                    .map(|c| c.to_rgba_f32())
                                    .unwrap_or([0.0, 0.0, 0.0, 1.0]),
                                bold: true,
                                italic: styled_text.italic,
                                underline: *level <= 2,
                                strikethrough: styled_text.strikethrough,
                                dim: false,
                                reverse: false,
                                blink: false,
                                wide: ch.width().unwrap_or(1) > 1,
                                double_height: false,
                                dirty: true,
                            });
                            current_col += ch.width().unwrap_or(1);
                        }
                    }

                    current_row += 1;
                    current_col = 0;
                }

                MarkdownElement::Paragraph(content) => {
                    // Add spacing before paragraph if not first element
                    if current_row > 0 {
                        current_row += 1;
                        current_col = 0;
                    }

                    // Join all styled text and wrap
                    let full_text: String = content.iter().map(|st| &st.text).collect::<String>();
                    let wrapped_lines = self.wrap_text(&full_text, self.context.terminal_width);

                    for line in wrapped_lines {
                        current_col = 0;
                        for ch in line.chars() {
                            if current_col >= self.context.terminal_width {
                                current_row += 1;
                                current_col = 0;
                            }
                            cells.push(TerminalCell {
                                character: ch,
                                foreground: [0.9, 0.9, 0.9, 1.0], // Light gray
                                background: [0.0, 0.0, 0.0, 1.0],
                                bold: false,
                                italic: false,
                                underline: false,
                                strikethrough: false,
                                dim: false,
                                reverse: false,
                                blink: false,
                                wide: ch.width().unwrap_or(1) > 1,
                                double_height: false,
                                dirty: true,
                            });
                            current_col += ch.width().unwrap_or(1);
                        }
                        current_row += 1;
                    }
                }

                MarkdownElement::CodeBlock { language, content } => {
                    // Add spacing before code block
                    if current_row > 0 {
                        current_row += 1;
                        current_col = 0;
                    }

                    // Highlight the code
                    let highlighted = self.highlight_code(content, language.as_deref())?;
                    
                    // Render with background and optional line numbers
                    let lines: Vec<&str> = content.lines().collect();
                    let line_number_width = if self.context.show_line_numbers {
                        format!("{}", lines.len()).len() + 1
                    } else {
                        0
                    };

                    for (line_idx, line) in lines.iter().enumerate() {
                        current_col = 0;
                        
                        // Render line number if enabled
                        if self.context.show_line_numbers {
                            let line_number = format!("{:>width$} ", line_idx + 1, width = line_number_width - 1);
                            for ch in line_number.chars() {
                                cells.push(TerminalCell {
                                    character: ch,
                                    foreground: [0.5, 0.5, 0.5, 1.0], // Gray
                                    background: [0.1, 0.1, 0.1, 1.0], // Dark gray
                                    bold: false,
                                    italic: false,
                                    underline: false,
                                    strikethrough: false,
                                    dim: true,
                                    reverse: false,
                                    blink: false,
                                    wide: false,
                                    double_height: false,
                                    dirty: true,
                                });
                                current_col += 1;
                            }
                        }

                        // Render code line
                        for ch in line.chars() {
                            if current_col >= self.context.terminal_width && !self.context.wrap_code {
                                break; // Truncate if wrapping disabled
                            }
                            if current_col >= self.context.terminal_width {
                                current_row += 1;
                                current_col = if self.context.show_line_numbers { line_number_width } else { 0 };
                            }

                            cells.push(TerminalCell {
                                character: ch,
                                foreground: [0.8, 0.8, 0.8, 1.0], // Light gray (fallback)
                                background: [0.1, 0.1, 0.1, 1.0], // Dark background
                                bold: false,
                                italic: false,
                                underline: false,
                                strikethrough: false,
                                dim: false,
                                reverse: false,
                                blink: false,
                                wide: ch.width().unwrap_or(1) > 1,
                                double_height: false,
                                dirty: true,
                            });
                            current_col += ch.width().unwrap_or(1);
                        }
                        current_row += 1;
                    }
                }

                MarkdownElement::List { ordered, items } => {
                    // Add spacing before list
                    if current_row > 0 {
                        current_row += 1;
                        current_col = 0;
                    }

                    for (idx, item) in items.iter().enumerate() {
                        current_col = 0;
                        
                        // Render list marker
                        let marker = if *ordered {
                            format!("{}. ", idx + 1)
                        } else {
                            "• ".to_string()
                        };

                        for ch in marker.chars() {
                            cells.push(TerminalCell {
                                character: ch,
                                foreground: [0.7, 0.7, 0.7, 1.0], // Gray
                                background: [0.0, 0.0, 0.0, 1.0],
                                bold: true,
                                italic: false,
                                underline: false,
                                strikethrough: false,
                                dim: false,
                                reverse: false,
                                blink: false,
                                wide: false,
                                double_height: false,
                                dirty: true,
                            });
                            current_col += ch.width().unwrap_or(1);
                        }

                        // Render list item content
                        for styled_text in item {
                            for ch in styled_text.text.chars() {
                                if current_col >= self.context.terminal_width {
                                    current_row += 1;
                                    current_col = marker.width(); // Indent continuation
                                }
                                cells.push(TerminalCell {
                                    character: ch,
                                    foreground: styled_text.color.to_rgba_f32(),
                                    background: styled_text.background
                                        .as_ref()
                                        .map(|c| c.to_rgba_f32())
                                        .unwrap_or([0.0, 0.0, 0.0, 1.0]),
                                    bold: styled_text.bold,
                                    italic: styled_text.italic,
                                    underline: styled_text.underline,
                                    strikethrough: styled_text.strikethrough,
                                    dim: false,
                                    reverse: false,
                                    blink: false,
                                    wide: ch.width().unwrap_or(1) > 1,
                                    double_height: false,
                                    dirty: true,
                                });
                                current_col += ch.width().unwrap_or(1);
                            }
                        }
                        current_row += 1;
                    }
                }

                MarkdownElement::ThematicBreak => {
                    // Add spacing before rule
                    if current_row > 0 {
                        current_row += 1;
                        current_col = 0;
                    }

                    // Render horizontal rule
                    let rule_char = '─'; // Unicode box drawing character
                    for col in 0..self.context.terminal_width.min(80) {
                        cells.push(TerminalCell {
                            character: rule_char,
                            foreground: [0.5, 0.5, 0.5, 1.0], // Gray
                            background: [0.0, 0.0, 0.0, 1.0],
                            bold: false,
                            italic: false,
                            underline: false,
                            strikethrough: false,
                            dim: false,
                            reverse: false,
                            blink: false,
                            wide: false,
                            double_height: false,
                            dirty: true,
                        });
                        current_col = col + 1;
                    }
                    current_row += 1;
                    current_col = 0;
                }

                MarkdownElement::LineBreak => {
                    current_row += 1;
                    current_col = 0;
                }

                // Handle other elements (simplified for now)
                _ => {
                    // Placeholder for unimplemented elements
                    current_row += 1;
                    current_col = 0;
                }
            }
        }

        self.performance_stats.total_render_time += start_time.elapsed();
        self.performance_stats.render_calls += 1;

        Ok(cells)
    }

    /// Wrap text to fit terminal width while preserving word boundaries
    fn wrap_text(&self, text: &str, width: usize) -> Vec<String> {
        if text.is_empty() {
            return vec![];
        }

        let options = Options::new(width)
            .break_words(false)
            .word_separator(textwrap::WordSeparator::UnicodeBreakProperties);

        wrap(text, &options).into_iter().map(|s| s.to_string()).collect()
    }

    /// Update rendering context (terminal resize, theme change, etc.)
    pub fn update_context(&mut self, context: RenderContext) -> Result<(), MarkdownError> {
        let theme_changed = context.code_theme != self.context.code_theme;
        self.context = context;

        if theme_changed {
            self.current_theme = Arc::new(
                self.theme_set
                    .themes
                    .get(&self.context.code_theme)
                    .or_else(|| self.theme_set.themes.get("base16-ocean.dark"))
                    .or_else(|| self.theme_set.themes.values().next())
                    .ok_or_else(|| MarkdownError::Highlighting("No themes available".to_string()))?
                    .clone(),
            );
            // Clear highlighter cache when theme changes
            self.highlighter_cache.clear();
        }

        Ok(())
    }

    /// Get performance statistics
    pub fn get_performance_stats(&self) -> &PerformanceStats {
        &self.performance_stats
    }

    /// Reset performance statistics
    pub fn reset_performance_stats(&mut self) {
        self.performance_stats = PerformanceStats::default();
    }

    /// Get list of available syntax highlighting languages
    pub fn get_available_languages(&self) -> Vec<String> {
        self.syntax_set
            .syntaxes()
            .iter()
            .flat_map(|syntax| {
                std::iter::once(syntax.name.clone())
                    .chain(syntax.file_extensions.iter().cloned())
            })
            .collect()
    }

    /// Get list of available themes
    pub fn get_available_themes(&self) -> Vec<String> {
        self.theme_set.themes.keys().cloned().collect()
    }

    /// Clear streaming buffer (useful for session resets)
    pub fn clear_streaming_buffer(&mut self) {
        self.streaming_buffer.clear();
        self.partial_elements.clear();
    }

    /// Check if renderer supports a language for syntax highlighting
    pub fn supports_language(&self, language: &str) -> bool {
        self.syntax_set.find_syntax_by_token(language).is_some()
            || self.syntax_set.find_syntax_by_extension(language).is_some()
    }
}

/// High-level interface for rendering markdown content to terminal grid
pub struct MarkdownTerminalRenderer {
    renderer: MarkdownRenderer,
    grid_width: usize,
    grid_height: usize,
    scroll_offset: usize,
}

impl MarkdownTerminalRenderer {
    pub fn new(context: RenderContext, grid_width: usize, grid_height: usize) -> Result<Self, MarkdownError> {
        let renderer = MarkdownRenderer::new(context)?;
        
        Ok(Self {
            renderer,
            grid_width,
            grid_height,
            scroll_offset: 0,
        })
    }

    /// Render markdown content to a terminal grid
    pub fn render_to_grid(&mut self, content: &str, grid: &mut TerminalGrid) -> Result<(), MarkdownError> {
        // Parse markdown
        let elements = self.renderer.parse_markdown(content)?;
        
        // Render to cells
        let cells = self.renderer.render_to_cells(&elements)?;
        
        // Clear the grid
        for row in 0..grid.height {
            for col in 0..grid.width {
                if let Some(cell) = grid.get_cell(col, row) {
                    let mut cleared_cell = cell.clone();
                    cleared_cell.character = ' ';
                    cleared_cell.foreground = [1.0, 1.0, 1.0, 1.0];
                    cleared_cell.background = [0.0, 0.0, 0.0, 1.0];
                    cleared_cell.bold = false;
                    cleared_cell.italic = false;
                    cleared_cell.underline = false;
                    cleared_cell.strikethrough = false;
                    grid.set_cell(col, row, cleared_cell);
                }
            }
        }
        
        // Apply cells to grid with scrolling
        for (idx, cell) in cells.iter().enumerate() {
            let row = (idx / self.grid_width) as u32;
            let col = (idx % self.grid_width) as u32;
            
            // Apply scrolling offset
            if row >= self.scroll_offset as u32 && 
               row < (self.scroll_offset + self.grid_height) as u32 {
                let display_row = row - self.scroll_offset as u32;
                if display_row < grid.height && col < grid.width {
                    grid.set_cell(col, display_row, cell.clone());
                }
            }
        }
        
        Ok(())
    }

    /// Render streaming markdown content
    pub fn render_streaming(&mut self, chunk: &str, grid: &mut TerminalGrid) -> Result<(), MarkdownError> {
        let elements = self.renderer.parse_streaming(chunk)?;
        
        if !elements.is_empty() {
            let cells = self.renderer.render_to_cells(&elements)?;
            
            // Apply cells to grid (similar to render_to_grid but incremental)
            for (idx, cell) in cells.iter().enumerate() {
                let row = (idx / self.grid_width) as u32;
                let col = (idx % self.grid_width) as u32;
                
                if row >= self.scroll_offset as u32 && 
                   row < (self.scroll_offset + self.grid_height) as u32 {
                    let display_row = row - self.scroll_offset as u32;
                    if display_row < grid.height && col < grid.width {
                        grid.set_cell(col, display_row, cell.clone());
                    }
                }
            }
        }
        
        Ok(())
    }

    /// Scroll the view
    pub fn scroll(&mut self, delta: i32) {
        if delta < 0 {
            let abs_delta = (-delta) as usize;
            self.scroll_offset = self.scroll_offset.saturating_sub(abs_delta);
        } else {
            self.scroll_offset += delta as usize;
        }
    }

    /// Set absolute scroll position
    pub fn set_scroll(&mut self, position: usize) {
        self.scroll_offset = position;
    }

    /// Get current scroll position
    pub fn get_scroll(&self) -> usize {
        self.scroll_offset
    }

    /// Update renderer context
    pub fn update_context(&mut self, context: RenderContext) -> Result<(), MarkdownError> {
        self.renderer.update_context(context)
    }

    /// Resize the grid
    pub fn resize(&mut self, width: usize, height: usize) {
        self.grid_width = width;
        self.grid_height = height;
        
        // Update renderer context
        let mut context = self.renderer.context.clone();
        context.terminal_width = width;
        let _ = self.renderer.update_context(context);
    }

    /// Get performance statistics
    pub fn get_performance_stats(&self) -> &PerformanceStats {
        self.renderer.get_performance_stats()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_markdown_parsing() {
        let mut renderer = MarkdownRenderer::new(RenderContext::default()).unwrap();
        
        let content = r#"# Header 1

This is a paragraph with **bold** and *italic* text.

```rust
fn main() {
    println!("Hello, world!");
}
```

- List item 1
- List item 2
"#;

        let elements = renderer.parse_markdown(content).unwrap();
        assert!(!elements.is_empty());
        
        // Check that we have the expected element types
        assert!(matches!(elements[0], MarkdownElement::Header { level: 1, .. }));
    }

    #[test]
    fn test_code_highlighting() {
        let mut renderer = MarkdownRenderer::new(RenderContext::default()).unwrap();
        
        let code = "fn main() {\n    println!(\"Hello\");\n}";
        let highlighted = renderer.highlight_code(code, Some("rust")).unwrap();
        
        assert!(!highlighted.is_empty());
    }

    #[test]
    fn test_streaming_parser() {
        let mut renderer = MarkdownRenderer::new(RenderContext::default()).unwrap();
        
        // Simulate streaming chunks
        let chunk1 = "# Header";
        let chunk2 = "\n\nParagraph text";
        
        let elements1 = renderer.parse_streaming(chunk1).unwrap();
        let elements2 = renderer.parse_streaming(chunk2).unwrap();
        
        // Should handle partial content gracefully
        assert!(elements1.len() + elements2.len() > 0);
    }
}