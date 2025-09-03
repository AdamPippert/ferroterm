use ferroterm::tty::{PtyConfig, TtyEngine};
use std::sync::Arc;
use std::time::Instant;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("TTY Engine Performance Benchmarks\n");

    // Test 1: PTY Creation Performance
    println!("=== PTY Creation Performance ===");
    let engine = TtyEngine::new();
    let config = PtyConfig::default();

    let mut creation_times = Vec::new();
    for i in 0..10 {
        let start = Instant::now();
        let pty_id = engine.create_pty(config.clone()).await?;
        let creation_time = start.elapsed();
        creation_times.push(creation_time);

        println!("PTY {} created in {:?}", i + 1, creation_time);
        engine.destroy_pty(pty_id).await?;
    }

    let avg_creation = creation_times.iter().sum::<Duration>() / creation_times.len() as u32;
    println!("Average creation time: {:?}", avg_creation);

    // Test 2: I/O Throughput
    println!("\n=== I/O Throughput Performance ===");
    let pty_id = engine.create_pty(config.clone()).await?;

    // Write throughput test
    let test_data = b"echo 'performance test data'\n";
    let iterations = 1000;

    let start = Instant::now();
    for _ in 0..iterations {
        engine.write_to_pty(pty_id, test_data).await?;
    }
    let write_duration = start.elapsed();

    let bytes_per_sec = (test_data.len() * iterations) as f64 / write_duration.as_secs_f64();
    println!("Write throughput: {:.2} bytes/sec", bytes_per_sec);
    println!(
        "Write latency: {:?} per operation",
        write_duration / iterations as u32
    );

    // Read throughput test
    sleep(Duration::from_millis(100)).await; // Let data accumulate

    let mut total_read = 0;
    let start = Instant::now();
    for _ in 0..100 {
        let mut buffer = [0u8; 1024];
        match engine.read_from_pty(pty_id, &mut buffer).await {
            Ok(bytes_read) => total_read += bytes_read,
            Err(_) => break,
        }
        if total_read == 0 {
            break;
        }
    }
    let read_duration = start.elapsed();

    if total_read > 0 {
        let read_bytes_per_sec = total_read as f64 / read_duration.as_secs_f64();
        println!("Read throughput: {:.2} bytes/sec", read_bytes_per_sec);
        println!("Total bytes read: {}", total_read);
    }

    engine.destroy_pty(pty_id).await?;

    // Test 3: Concurrent PTY Management
    println!("\n=== Concurrent PTY Management ===");
    let engine = Arc::new(TtyEngine::new());
    let concurrent_sessions = 20;

    let start = Instant::now();
    let mut handles = Vec::new();

    for i in 0..concurrent_sessions {
        let engine_clone = Arc::clone(&engine);
        let config_clone = config.clone();

        let handle = tokio::spawn(async move {
            let pty_id = engine_clone.create_pty(config_clone).await.unwrap();

            // Perform some I/O
            let data = format!("echo concurrent_test_{}\n", i);
            engine_clone
                .write_to_pty(pty_id, data.as_bytes())
                .await
                .unwrap();

            // Small delay to simulate real usage
            sleep(Duration::from_millis(10)).await;

            engine_clone.destroy_pty(pty_id).await.unwrap();
        });

        handles.push(handle);
    }

    // Wait for all to complete
    for handle in handles {
        handle.await?;
    }

    let concurrent_duration = start.elapsed();
    println!(
        "Created and destroyed {} PTYs concurrently in {:?}",
        concurrent_sessions, concurrent_duration
    );
    println!(
        "Average per PTY: {:?}",
        concurrent_duration / concurrent_sessions
    );

    // Test 4: Signal Handling Performance
    println!("\n=== Signal Handling Performance ===");
    let pty_id = engine.create_pty(config.clone()).await?;

    let signal_count = 100;
    let start = Instant::now();

    for _ in 0..signal_count {
        engine
            .send_signal(Some(pty_id), nix::sys::signal::Signal::SIGWINCH)
            .await?;
    }

    let signal_duration = start.elapsed();
    println!("Sent {} signals in {:?}", signal_count, signal_duration);
    println!(
        "Average signal latency: {:?}",
        signal_duration / signal_count
    );

    engine.destroy_pty(pty_id).await?;

    // Test 5: Memory Usage and Stats
    println!("\n=== Memory and Statistics ===");
    let stats = engine.get_stats();
    println!("Engine statistics:");
    println!("  Sessions created: {}", stats.sessions_created);
    println!("  Sessions destroyed: {}", stats.sessions_destroyed);
    println!("  Total bytes read: {}", stats.total_bytes_read);
    println!("  Total bytes written: {}", stats.total_bytes_written);
    println!("  Signals sent: {}", stats.signal_count);
    println!("  Errors: {}", stats.errors);

    // Test 6: Stress Test
    println!("\n=== Stress Test ===");
    let stress_sessions = 50;
    let mut pty_ids = Vec::new();

    let start = Instant::now();

    // Create many sessions
    for _ in 0..stress_sessions {
        let pty_id = engine.create_pty(config.clone()).await?;
        pty_ids.push(pty_id);
    }

    println!(
        "Created {} PTY sessions in {:?}",
        stress_sessions,
        start.elapsed()
    );
    println!("Active sessions: {}", engine.get_session_count());

    // Perform I/O on all sessions
    let io_start = Instant::now();
    for &pty_id in &pty_ids {
        let _ = engine.write_to_pty(pty_id, b"echo stress_test\n").await;
    }
    println!("Performed I/O on all sessions in {:?}", io_start.elapsed());

    // Clean up
    let cleanup_start = Instant::now();
    for pty_id in pty_ids {
        engine.destroy_pty(pty_id).await?;
    }
    println!("Cleaned up all sessions in {:?}", cleanup_start.elapsed());

    // Verify performance requirements
    println!("\n=== Performance Requirements Check ===");

    if avg_creation.as_millis() <= 10 {
        println!(
            "✓ PTY creation time: {:?} (requirement: ≤10ms)",
            avg_creation
        );
    } else {
        println!(
            "✗ PTY creation time: {:?} (requirement: ≤10ms)",
            avg_creation
        );
    }

    let write_latency = write_duration / iterations as u32;
    if write_latency.as_millis() <= 1 {
        println!("✓ Write latency: {:?} (requirement: ≤1ms)", write_latency);
    } else {
        println!("✗ Write latency: {:?} (requirement: ≤1ms)", write_latency);
    }

    let signal_latency = signal_duration / signal_count;
    if signal_latency.as_micros() <= 1000 {
        println!("✓ Signal latency: {:?} (requirement: ≤1ms)", signal_latency);
    } else {
        println!("✗ Signal latency: {:?} (requirement: ≤1ms)", signal_latency);
    }

    if engine.get_session_count() == 0 {
        println!("✓ No PTY leaks detected");
    } else {
        println!(
            "✗ PTY leak detected: {} sessions remaining",
            engine.get_session_count()
        );
    }

    println!("\nBenchmark completed successfully!");
    Ok(())
}
