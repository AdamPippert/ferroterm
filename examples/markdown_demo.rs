use ferroterm::markdown_renderer::{MarkdownTerminalRenderer, RenderContext};
use ferroterm::renderer::TerminalGrid;
use std::io::{self, Write};
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Ferroterm Markdown Renderer Demo");
    println!("================================\n");

    // Create render context with terminal capabilities
    let context = RenderContext {
        terminal_width: 80,
        supports_truecolor: true,
        supports_256color: true,
        supports_unicode: true,
        tab_width: 4,
        code_theme: "base16-ocean.dark".to_string(),
        wrap_code: true,
        show_line_numbers: true,
    };

    // Create markdown renderer
    let mut renderer = MarkdownTerminalRenderer::new(context, 80, 24)?;

    // Sample markdown content
    let markdown_content = r#"# Welcome to Ferroterm

Ferroterm is a **high-performance** terminal emulator with *advanced* markdown rendering capabilities.

## Features

- 🚀 **Fast**: GPU-accelerated rendering
- 🎨 **Beautiful**: Syntax highlighting for 20+ languages  
- 📝 **Smart**: Real-time markdown processing
- ⚡ **Efficient**: Streaming content support

## Code Example

Here's a sample Rust function:

```rust
fn fibonacci(n: u32) -> u64 {
    match n {
        0 => 0,
        1 => 1,
        _ => fibonacci(n - 1) + fibonacci(n - 2),
    }
}

fn main() {
    println!("Fibonacci(10) = {}", fibonacci(10));
}
```

## Languages Supported

1. **Rust** - Systems programming
2. **Python** - Scripting and data science
3. **JavaScript** - Web development
4. **Go** - Backend services
5. **C++** - Performance-critical applications

## Tables

| Language | Performance | Memory Safety | Ease of Use |
|----------|-------------|---------------|-------------|
| Rust     | Excellent   | Excellent     | Moderate    |
| C++      | Excellent   | Poor          | Difficult   |
| Python   | Moderate    | Good          | Excellent   |
| Go       | Good        | Good          | Good        |

## Blockquotes

> "The best way to predict the future is to create it."
> 
> — Peter Drucker

---

## Interactive Elements

Click on links: [GitHub](https://github.com) | [Documentation](https://docs.rs)

`inline code` and **emphasis** work seamlessly together.

### Performance Metrics

- Parse time: < 10ms per KB
- Render time: < 5ms per screen
- Memory usage: < 1MB per document
- Languages: 20+ supported

**Thank you for using Ferroterm!**
"#;

    // Create a terminal grid
    let mut grid = TerminalGrid::new(80, 40);

    // Benchmark rendering
    let start_time = Instant::now();
    renderer.render_to_grid(markdown_content, &mut grid)?;
    let render_time = start_time.elapsed();

    println!("Rendered markdown in {:?}", render_time);
    println!("Content size: {} bytes", markdown_content.len());
    
    // Get performance stats
    if let Some(stats) = renderer.get_performance_stats() {
        println!("Performance stats:");
        println!("  Total parse time: {:?}", stats.total_parse_time);
        println!("  Total render time: {:?}", stats.total_render_time);
        println!("  Bytes processed: {}", stats.total_bytes_processed);
        println!("  Render calls: {}", stats.render_calls);
    }

    println!("\nGrid dimensions: {}x{}", grid.width, grid.height);
    println!("Total cells: {}", grid.cells.len());
    
    // Display a sample of the rendered grid
    println!("\n--- Rendered Output Preview (first 10 lines) ---");
    for y in 0..10.min(grid.height) {
        for x in 0..grid.width {
            if let Some(cell) = grid.get_cell(x, y) {
                // Convert to terminal character (simplified)
                let mut ch = cell.character;
                if ch == '\0' || ch == '\u{0}' {
                    ch = ' ';
                }
                print!("{}", ch);
            }
        }
        println!();
    }

    println!("\n--- Streaming Demo ---");
    
    // Demonstrate streaming parsing
    let streaming_chunks = vec![
        "# Streaming",
        " Example\n\n",
        "This content is being ",
        "**streamed** in real-time.\n\n",
        "```python\n",
        "def hello():\n",
        "    print('Hello, ",
        "streaming world!')\n",
        "```\n",
    ];

    let mut streaming_grid = TerminalGrid::new(80, 20);
    
    for (i, chunk) in streaming_chunks.iter().enumerate() {
        println!("Processing chunk {}: '{}'", i + 1, chunk.escape_debug());
        let chunk_start = Instant::now();
        renderer.render_streaming(chunk, &mut streaming_grid)?;
        let chunk_time = chunk_start.elapsed();
        println!("  Processed in {:?}", chunk_time);
        
        // Small delay to simulate real streaming
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    println!("\n--- Language Support Demo ---");
    
    // Test different programming languages
    let languages = vec![
        ("rust", "fn main() { println!(\"Hello Rust!\"); }"),
        ("python", "def hello():\n    print(\"Hello Python!\")"),
        ("javascript", "function hello() { console.log('Hello JS!'); }"),
        ("go", "func main() {\n    fmt.Println(\"Hello Go!\")\n}"),
        ("c", "#include <stdio.h>\nint main() { printf(\"Hello C!\\n\"); }"),
    ];

    for (lang, code) in languages {
        let code_block = format!("```{}\n{}\n```", lang, code);
        let mut lang_grid = TerminalGrid::new(60, 10);
        
        let lang_start = Instant::now();
        renderer.render_to_grid(&code_block, &mut lang_grid)?;
        let lang_time = lang_start.elapsed();
        
        println!("{}: rendered in {:?}", lang, lang_time);
    }

    println!("\n--- Theme Demo ---");
    
    // Test different themes
    let themes = vec!["base16-ocean.dark", "base16-eighties.dark", "base16-mocha.dark"];
    
    for theme in themes {
        let mut theme_context = RenderContext::default();
        theme_context.code_theme = theme.to_string();
        theme_context.terminal_width = 60;
        
        let mut theme_renderer = MarkdownTerminalRenderer::new(theme_context, 60, 15)?;
        let theme_code = format!("# {} Theme\n\n```rust\nfn themed_code() {{\n    // This is styled with {}\n}}\n```", theme, theme);
        
        let mut theme_grid = TerminalGrid::new(60, 15);
        let theme_start = Instant::now();
        theme_renderer.render_to_grid(&theme_code, &mut theme_grid)?;
        let theme_time = theme_start.elapsed();
        
        println!("{}: rendered in {:?}", theme, theme_time);
    }

    println!("\n--- Unicode and Emoji Support ---");
    
    let unicode_content = r#"# Unicode Support 🎉

Ferroterm supports full Unicode rendering:

## Various Scripts
- **English**: Hello World
- **中文**: 你好世界  
- **日本語**: こんにちは世界
- **العربية**: مرحبا بالعالم
- **Русский**: Привет мир
- **Emoji**: 🦀 🚀 ⚡ 🎨 💻 🌟

## Box Drawing
┌─────────────────┐
│   Terminal Box  │
│  ┌───────────┐  │
│  │ Nested    │  │  
│  └───────────┘  │
└─────────────────┘

## Mathematical Symbols
∀x ∈ ℝ: x² ≥ 0
π ≈ 3.14159
∑(n=1 to ∞) 1/n² = π²/6
"#;

    let mut unicode_grid = TerminalGrid::new(80, 25);
    let unicode_start = Instant::now();
    renderer.render_to_grid(unicode_content, &mut unicode_grid)?;
    let unicode_time = unicode_start.elapsed();
    
    println!("Unicode content rendered in {:?}", unicode_time);

    println!("\n--- Scrolling Demo ---");
    
    // Demonstrate scrolling functionality
    let long_content = (0..50)
        .map(|i| format!("## Section {}\n\nThis is paragraph {} with some content to demonstrate scrolling.\n", i + 1, i + 1))
        .collect::<Vec<_>>()
        .join("\n");

    let mut scroll_grid = TerminalGrid::new(80, 10);
    renderer.render_to_grid(&long_content, &mut scroll_grid)?;
    
    println!("Initial position (scroll offset: {})", renderer.get_scroll());
    
    // Scroll down
    renderer.scroll(5);
    renderer.render_to_grid(&long_content, &mut scroll_grid)?;
    println!("After scrolling down 5 lines (scroll offset: {})", renderer.get_scroll());
    
    // Scroll back up
    renderer.scroll(-3);
    renderer.render_to_grid(&long_content, &mut scroll_grid)?;
    println!("After scrolling up 3 lines (scroll offset: {})", renderer.get_scroll());

    println!("\n--- Performance Stress Test ---");
    
    // Generate large content for stress testing
    let stress_content = (0..100)
        .map(|i| {
            format!(
                r#"# Section {}

This is a **performance** test section with `inline code` and *emphasis*.

```rust
// Code block {}
fn test_function_{}() {{
    let value = {};
    println!("Processing section {{}}", value);
    
    // Some complex logic here
    for i in 0..{} {{
        if i % 2 == 0 {{
            println!("Even: {{}}", i);
        }} else {{
            println!("Odd: {{}}", i);
        }}
    }}
}}
```

1. List item {} - one
2. List item {} - two  
3. List item {} - three

> Quote for section {}
> 
> This demonstrates blockquote rendering.

---
"#,
                i + 1, i + 1, i + 1, (i + 1) * 10, i + 1, (i + 1) * 5,
                i + 1, i + 1, i + 1, i + 1
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let mut stress_grid = TerminalGrid::new(100, 50);
    let stress_start = Instant::now();
    renderer.render_to_grid(&stress_content, &mut stress_grid)?;
    let stress_time = stress_start.elapsed();

    println!("Stress test results:");
    println!("  Content size: {} bytes", stress_content.len());
    println!("  Render time: {:?}", stress_time);
    println!("  Throughput: {:.2} KB/s", stress_content.len() as f64 / stress_time.as_secs_f64() / 1024.0);
    
    // Final performance stats
    if let Some(stats) = renderer.get_performance_stats() {
        println!("\nFinal Performance Statistics:");
        println!("  Total parse time: {:?}", stats.total_parse_time);
        println!("  Total render time: {:?}", stats.total_render_time);
        println!("  Total bytes processed: {} bytes", stats.total_bytes_processed);
        println!("  Total render calls: {}", stats.render_calls);
        
        if stats.render_calls > 0 {
            println!("  Average parse time per call: {:?}", 
                     stats.total_parse_time / stats.render_calls as u32);
            println!("  Average render time per call: {:?}", 
                     stats.total_render_time / stats.render_calls as u32);
            println!("  Average bytes per call: {:.2}", 
                     stats.total_bytes_processed as f64 / stats.render_calls as f64);
        }
    }

    println!("\n🎉 Markdown renderer demo completed successfully!");
    Ok(())
}