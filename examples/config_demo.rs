use pachyterm::config::{Config, ConfigManager};
use std::fs;
use tempfile::TempDir;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Ferroterm Configuration System Demo\n");

    // Create a temporary directory for demo
    let temp_dir = TempDir::new()?;
    let config_path = temp_dir.path().join("ferroterm.toml");

    // Demo 1: Default configuration
    println!("1. Creating default configuration...");
    let default_config = Config::default();
    println!("   Font size: {}", default_config.ui.font_size);
    println!("   Theme: {}", default_config.ui.theme);
    println!("   Agent prefix: '{}'", default_config.keymap.prefix);
    println!("   Default model: {}", default_config.agent.default_model);

    // Demo 2: Loading from file (will create default if missing)
    println!("\n2. Loading configuration from file...");
    let _config = ConfigManager::load_config_from_path(&config_path)?;
    println!("   Configuration loaded successfully!");
    println!("   Config file created at: {}", config_path.display());

    // Demo 3: Custom configuration
    println!("\n3. Creating custom configuration...");
    let custom_config = r#"# Custom Ferroterm Configuration
[ui]
font_size = 16
font_family = "Fira Code"
theme = "light"
cursor_style = "beam"

[keymap]
prefix = "ai"
[keymap.bindings]
"ctrl+c" = "interrupt"
"ctrl+shift+c" = "copy"

[agent]
default_model = "gpt-4"
temperature = 0.3
context_lines = 50

[[models.models]]
name = "gpt-4"
api_endpoint = "https://api.openai.com/v1"
context_window = 8192

[telemetry]
enabled = true
"#;

    fs::write(&config_path, custom_config)?;
    let loaded_config = ConfigManager::load_config_from_path(&config_path)?;

    println!("   Custom font size: {}", loaded_config.ui.font_size);
    println!("   Custom theme: {}", loaded_config.ui.theme);
    println!("   Custom prefix: '{}'", loaded_config.keymap.prefix);
    println!("   Custom model: {}", loaded_config.agent.default_model);
    println!("   Telemetry enabled: {}", loaded_config.telemetry.enabled);

    // Demo 4: Configuration manager with live reload
    println!("\n4. Testing configuration manager...");
    let manager = ConfigManager::from_path(config_path.clone())?;

    println!(
        "   Current font size: {}",
        manager.get_config().ui.font_size
    );

    // Modify config file
    let updated_config = custom_config.replace("font_size = 16", "font_size = 20");
    fs::write(&config_path, updated_config)?;

    // Manual reload
    manager.reload_config()?;
    println!(
        "   Updated font size: {}",
        manager.get_config().ui.font_size
    );

    // Demo 5: Error handling
    println!("\n5. Testing error handling...");
    let invalid_config = r#"
[ui]
font_size = 999  # This will fail validation (must be ≤72)
"#;

    fs::write(&config_path, invalid_config)?;
    match ConfigManager::load_config_from_path(&config_path) {
        Ok(_) => println!("   Unexpected success with invalid config"),
        Err(e) => println!("   Correctly caught error: {}", e),
    }

    println!("\nDemo completed successfully!");
    Ok(())
}
