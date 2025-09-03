use ferroterm::config::{Config, ConfigManager};
use std::fs;
use std::time::Instant;
use tempfile::TempDir;

fn main() {
    println!("Running configuration performance benchmarks...\n");

    // Test 1: Default config creation
    let start = Instant::now();
    let _config = Config::default();
    let default_time = start.elapsed();
    println!("Default config creation: {:?}", default_time);

    // Test 2: Small config file load
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("small.toml");
    let small_config = r#"
[ui]
font_size = 14
theme = "dark"

[agent]
temperature = 0.8
"#;
    fs::write(&config_path, small_config).unwrap();

    let start = Instant::now();
    let _config = ConfigManager::load_config_from_path(&config_path).unwrap();
    let small_load_time = start.elapsed();
    println!("Small config load: {:?}", small_load_time);

    // Test 3: Large config file load
    let large_config_path = temp_dir.path().join("large.toml");
    let large_config = generate_large_config();
    fs::write(&large_config_path, large_config).unwrap();

    let start = Instant::now();
    let _config = ConfigManager::load_config_from_path(&large_config_path).unwrap();
    let large_load_time = start.elapsed();
    println!("Large config load: {:?}", large_load_time);

    // Test 4: Multiple rapid loads (simulating hot path)
    let mut total_time = std::time::Duration::ZERO;
    const ITERATIONS: usize = 1000;

    for _ in 0..ITERATIONS {
        let start = Instant::now();
        let _config = ConfigManager::load_config_from_path(&config_path).unwrap();
        total_time += start.elapsed();
    }

    let avg_load_time = total_time / ITERATIONS as u32;
    println!(
        "Average load time ({} iterations): {:?}",
        ITERATIONS, avg_load_time
    );

    // Test 5: Config manager creation
    let start = Instant::now();
    let _manager = ConfigManager::new();
    let manager_creation_time = start.elapsed();
    println!("ConfigManager creation: {:?}", manager_creation_time);

    // Test 6: Memory usage estimation
    let config = Config::default();
    let config_size = std::mem::size_of_val(&config);
    println!("Config struct size: {} bytes", config_size);

    // Verify performance requirements
    println!("\n=== Performance Requirements Check ===");

    let load_time_ms = small_load_time.as_millis();
    if load_time_ms <= 10 {
        println!(
            "✓ Config load time: {}ms (requirement: ≤10ms)",
            load_time_ms
        );
    } else {
        println!(
            "✗ Config load time: {}ms (requirement: ≤10ms)",
            load_time_ms
        );
    }

    let large_load_time_ms = large_load_time.as_millis();
    if large_load_time_ms <= 10 {
        println!(
            "✓ Large config load time: {}ms (requirement: ≤10ms)",
            large_load_time_ms
        );
    } else {
        println!(
            "✗ Large config load time: {}ms (requirement: ≤10ms)",
            large_load_time_ms
        );
    }

    let avg_load_time_us = avg_load_time.as_micros();
    if avg_load_time_us <= 10000 {
        println!(
            "✓ Average load time: {}μs (requirement: ≤10ms)",
            avg_load_time_us
        );
    } else {
        println!(
            "✗ Average load time: {}μs (requirement: ≤10ms)",
            avg_load_time_us
        );
    }

    if config_size <= 1024 {
        println!("✓ Memory usage: {} bytes (requirement: ≤1KB)", config_size);
    } else {
        println!("✗ Memory usage: {} bytes (requirement: ≤1KB)", config_size);
    }
}

fn generate_large_config() -> String {
    let mut config = String::from(
        r#"
[ui]
font_size = 12
font_family = "JetBrains Mono"
theme = "dark"
cursor_style = "block"
line_height = 1.2
padding = 4

[keymap]
prefix = "f"
escape_sequence = "\\f"

[keymap.bindings]
"#,
    );

    // Add 200 keybindings
    for i in 0..200 {
        config.push_str(&format!("\"ctrl+alt+f{}\" = \"action{}\"\n", i, i));
    }

    config.push_str(
        r#"
[agent]
default_model = "mistral-7b-instruct"
context_lines = 100
timeout_ms = 30000
max_tokens = 2048
temperature = 0.7

[models]
cache_dir = "~/.cache/ferroterm/models"

"#,
    );

    // Add 100 models
    for i in 0..100 {
        config.push_str(&format!(
            r#"
[[models.models]]
name = "model{}"
path = "~/.cache/ferroterm/models/model{}.gguf"
quantization = "q4_0"
context_window = 4096
"#,
            i, i
        ));
    }

    config.push_str(
        r#"
[telemetry]
enabled = false
endpoint = "https://telemetry.ferroterm.dev"
batch_size = 100
flush_interval_ms = 60000
"#,
    );

    config
}
