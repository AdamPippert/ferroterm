use pachyterm::tty::{PtyConfig, TerminalMode, TtyEngine};
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Ferroterm TTY Engine Demo\n");

    // Create TTY engine
    let engine = TtyEngine::new();

    // Demo 1: Basic PTY creation and management
    println!("1. Creating PTY session...");
    let mut config = PtyConfig::default();
    config.shell = "/bin/bash".to_string();
    config.rows = 24;
    config.cols = 80;

    let pty_id = engine.create_pty(config).await?;
    println!("   PTY {} created successfully", pty_id);
    println!("   Active sessions: {}", engine.get_session_count());

    // Demo 2: Terminal I/O
    println!("\n2. Testing terminal I/O...");
    let command = b"echo 'Hello from Ferroterm TTY Engine!'\n";
    let bytes_written = engine.write_to_pty(pty_id, command).await?;
    println!("   Wrote {} bytes to PTY", bytes_written);

    // Give the command time to execute
    sleep(Duration::from_millis(100)).await;

    // Read the output
    let mut buffer = [0u8; 1024];
    match engine.read_from_pty(pty_id, &mut buffer).await {
        Ok(bytes_read) => {
            if bytes_read > 0 {
                let output = String::from_utf8_lossy(&buffer[..bytes_read]);
                println!("   Read {} bytes: {}", bytes_read, output.trim());
            } else {
                println!("   No output available yet");
            }
        }
        Err(e) => println!("   Read error: {}", e),
    }

    // Demo 3: Terminal modes
    println!("\n3. Testing terminal modes...");
    engine.set_pty_mode(pty_id, TerminalMode::Raw)?;
    println!("   Set to raw mode");

    engine.set_pty_mode(pty_id, TerminalMode::Cooked)?;
    println!("   Set to cooked mode");

    engine.set_pty_mode(pty_id, TerminalMode::AltScreen)?;
    println!("   Set to alternate screen mode");

    // Demo 4: Window resizing
    println!("\n4. Testing window resize...");
    engine.resize_pty(pty_id, 50, 120)?;
    println!("   Resized to 50x120");

    engine.resize_pty(pty_id, 24, 80)?;
    println!("   Resized back to 24x80");

    // Demo 5: Signal handling
    println!("\n5. Testing signal handling...");
    engine
        .send_signal(Some(pty_id), nix::sys::signal::Signal::SIGWINCH)
        .await?;
    println!("   Sent SIGWINCH signal");

    // Demo 6: Statistics
    println!("\n6. PTY statistics...");
    let (bytes_read, bytes_written, uptime) = engine.get_pty_stats(pty_id)?;
    println!("   Bytes read: {}", bytes_read);
    println!("   Bytes written: {}", bytes_written);
    println!("   Uptime: {:?}", uptime);

    // Demo 7: Multiple sessions
    println!("\n7. Testing multiple sessions...");
    let mut session_ids = Vec::new();

    for i in 0..3 {
        let config = PtyConfig::default();
        let session_id = engine.create_pty(config).await?;
        session_ids.push(session_id);
        println!("   Created session {}: PTY {}", i + 1, session_id);
    }

    println!("   Total active sessions: {}", engine.get_session_count());

    // Clean up additional sessions
    for session_id in session_ids {
        engine.destroy_pty(session_id).await?;
    }

    // Demo 8: Engine statistics
    println!("\n8. Engine statistics...");
    let stats = engine.get_stats();
    println!("   Sessions created: {}", stats.sessions_created);
    println!("   Sessions destroyed: {}", stats.sessions_destroyed);
    println!("   Total bytes read: {}", stats.total_bytes_read);
    println!("   Total bytes written: {}", stats.total_bytes_written);
    println!("   Signals sent: {}", stats.signal_count);
    println!("   Errors: {}", stats.errors);

    // Demo 9: Cleanup
    println!("\n9. Cleaning up...");
    engine.destroy_pty(pty_id).await?;
    println!("   PTY {} destroyed", pty_id);
    println!("   Active sessions: {}", engine.get_session_count());

    println!("\nDemo completed successfully!");
    println!("The TTY Engine provides:");
    println!("  ✓ Fast PTY creation and management");
    println!("  ✓ Efficient I/O operations");
    println!("  ✓ Signal handling");
    println!("  ✓ Terminal mode switching");
    println!("  ✓ Window resizing");
    println!("  ✓ Multiple concurrent sessions");
    println!("  ✓ Comprehensive statistics");
    println!("  ✓ Clean resource management");

    Ok(())
}
