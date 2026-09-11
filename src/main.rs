//! Fog Gateway Daemon - Main Entry Point
//!
//! Demonstrates:
//! - 1.3 Network Ingestion Subsystem (UDP socket + epoll reactor)
//! - 1.2 Wire Protocol & Serialization (32-byte frame parsing + CRC-16)

mod protocol;
mod ingestion;

use ingestion::{IngestionEngine, TelemetryHandler};
use protocol::{DroneTelemetry, ParseError};
use std::net::SocketAddr;

/// Example telemetry handler that accumulates frame data
struct LoggingHandler {
    frame_count: u64,
    last_frame: Option<DroneTelemetry>,
    device_ids: Vec<u32>,
}

impl LoggingHandler {
    fn new() -> Self {
        LoggingHandler {
            frame_count: 0,
            last_frame: None,
            device_ids: Vec::new(),
        }
    }

    fn device_count(&self) -> usize {
        self.device_ids.iter().collect::<std::collections::HashSet<_>>().len()
    }
}

impl TelemetryHandler for LoggingHandler {
    fn handle_frame(&mut self, frame: DroneTelemetry) {
        self.frame_count += 1;
        self.last_frame = Some(frame);

        // Copy packed struct fields to local variables to avoid alignment issues
        let device_id = frame.device_id;
        let latitude = frame.latitude;
        let longitude = frame.longitude;
        let timestamp = frame.timestamp;

        if !self.device_ids.contains(&device_id) {
            self.device_ids.push(device_id);
        }

        // Log every frame (remove % 1000 to see all)
        println!(
            "[Frame #{}] Device={}, Lat={:.4}, Lon={:.4}, Ts={}",
            self.frame_count, device_id, latitude, longitude, timestamp
        );
    }

    fn handle_error(&mut self, _source: SocketAddr, error: ParseError) {
        eprintln!("[Error] {}", error);
    }
}

fn main() {
    println!("╔═══════════════════════════════════════════════════════════════════╗");
    println!("║  Fog Gateway Daemon - Network Ingestion & Wire Protocol Demo    ║");
    println!("║  Components: 1.3 (Ingestion) + 1.2 (Protocol)                   ║");
    println!("╚═══════════════════════════════════════════════════════════════════╝\n");

    // Verify struct layout
    protocol::DroneTelemetry::verify_layout();
    println!("[✓] DroneTelemetry struct is exactly 32 bytes (repr(C, packed))\n");

    // Create ingestion engine
    let mut engine = IngestionEngine::new("127.0.0.1:8081")
        .expect("Failed to bind UDP socket");

    println!(
        "[✓] Bound UDP socket to {}\n",
        engine.bound_addr()
    );

    // Create handler
    let mut handler = LoggingHandler::new();

    println!("═══════════════════════════════════════════════════════════════════");
    println!("Listening for telemetry packets...");
    println!("Test with: python3 edge_streamer_redesigned.py");
    println!("═══════════════════════════════════════════════════════════════════\n");

    // Run ingestion loop
    // Runs indefinitely unless you Ctrl+C
    engine.run(&mut handler, None);

    // Print summary
    println!("\n═══════════════════════════════════════════════════════════════════");
    println!("Ingestion Summary");
    println!("═══════════════════════════════════════════════════════════════════");
    println!("Total frames received: {}", handler.frame_count);
    println!("Unique devices: {}", handler.device_count());
    if let Some(last) = handler.last_frame {
        // Copy packed struct fields to local variables to avoid alignment issues
        let device_id = last.device_id;
        let latitude = last.latitude;
        let longitude = last.longitude;
        println!(
            "Last frame: Device {} at ({:.4}, {:.4})",
            device_id, latitude, longitude
        );
    }
    let stats = engine.stats();
    println!("Valid packets: {}", stats.packets_valid);
    println!("Invalid packets: {}", stats.packets_invalid);
    println!("Error rate: {:.3}%", stats.error_rate() * 100.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_logging_handler() {
        let mut handler = LoggingHandler::new();

        // Create a test frame
        let frame = DroneTelemetry {
            magic: 0xF06F,
            device_id: 101,
            timestamp: 1234567890,
            latitude: 13.0827,
            longitude: 80.2707,
            checksum: 0,
        };

        handler.handle_frame(frame);
        assert_eq!(handler.frame_count, 1);
        assert_eq!(handler.device_count(), 1);
    }
}
