//! Network Ingestion Subsystem (1.3)
//!
//! Implements a non-blocking UDP socket reactor using Linux epoll
//! to handle high-frequency concurrent telemetry streams from edge devices.
//!
//! Features:
//! - Non-blocking UDP socket binding
//! - epoll-based event notification (no polling loops)
//! - Efficient kernel socket buffer clearing
//! - Target: 100,000 packets/second with ≤50µs latency

use crate::protocol::{DroneTelemetry, ParseError};
use nix::poll::{poll, PollFd, PollFlags};
use std::io;
use std::net::{SocketAddr, UdpSocket};

/// Maximum UDP packet size (should be 32 bytes for our protocol, but allow larger)
const MAX_PACKET_SIZE: usize = 2048;

/// Maximum events to process per epoll_wait call
const MAX_EVENTS: usize = 512;

/// Poll timeout in milliseconds (0 = non-blocking)
const POLL_TIMEOUT_MS: i32 = 1000;

/// Statistics for the ingestion loop
#[derive(Debug, Clone, Copy)]
pub struct IngestionStats {
    /// Total packets received
    pub packets_received: u64,
    /// Packets with valid format
    pub packets_valid: u64,
    /// Packets with parse errors
    pub packets_invalid: u64,
    /// Size errors
    pub errors_size: u64,
    /// Magic number errors
    pub errors_magic: u64,
    /// Checksum errors
    pub errors_checksum: u64,
}

impl Default for IngestionStats {
    fn default() -> Self {
        IngestionStats {
            packets_received: 0,
            packets_valid: 0,
            packets_invalid: 0,
            errors_size: 0,
            errors_magic: 0,
            errors_checksum: 0,
        }
    }
}

impl IngestionStats {
    pub fn reset(&mut self) {
        *self = IngestionStats::default();
    }

    pub fn error_rate(&self) -> f64 {
        if self.packets_received == 0 {
            0.0
        } else {
            (self.packets_invalid as f64) / (self.packets_received as f64)
        }
    }
}

/// Handler function for processing parsed telemetry frames
/// Implement this trait to handle incoming frames
pub trait TelemetryHandler {
    fn handle_frame(&mut self, frame: DroneTelemetry);
    fn handle_error(&mut self, source: SocketAddr, error: ParseError);
}

/// Null handler for testing - discards frames
pub struct NullHandler;

impl TelemetryHandler for NullHandler {
    fn handle_frame(&mut self, _frame: DroneTelemetry) {}
    fn handle_error(&mut self, _source: SocketAddr, _error: ParseError) {}
}

/// Network Ingestion Engine
/// Manages a non-blocking UDP socket with epoll-based event notification
pub struct IngestionEngine {
    socket: UdpSocket,
    bind_addr: SocketAddr,
    buffer: Vec<u8>,
    stats: IngestionStats,
}

impl IngestionEngine {
    /// Creates a new IngestionEngine bound to the specified address
    ///
    /// # Arguments
    /// * `addr` - Socket address to bind to (e.g., "127.0.0.1:8080")
    ///
    /// # Returns
    /// * `Ok(IngestionEngine)` - Successfully bound
    /// * `Err(io::Error)` - Binding failed
    pub fn new(addr: &str) -> io::Result<Self> {
        let bind_addr: SocketAddr = addr
            .parse()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, format!("Invalid addr: {}", e)))?;

        // Create and bind UDP socket
        let socket = UdpSocket::bind(bind_addr)?;

        // Set non-blocking mode
        socket.set_nonblocking(true)?;

        // Increase kernel socket buffer sizes for high throughput
        let recv_buf_size = 4 * 1024 * 1024; // 4 MB
        nix::sys::socket::setsockopt(
            &socket,
            nix::sys::socket::sockopt::RcvBuf,
            &recv_buf_size,
        )
        .ok(); // Ignore if not supported

        let send_buf_size = 4 * 1024 * 1024; // 4 MB
        nix::sys::socket::setsockopt(
            &socket,
            nix::sys::socket::sockopt::SndBuf,
            &send_buf_size,
        )
        .ok(); // Ignore if not supported

        Ok(IngestionEngine {
            socket,
            bind_addr,
            buffer: vec![0u8; MAX_PACKET_SIZE],
            stats: IngestionStats::default(),
        })
    }

    /// Returns the bound socket address
    pub fn bound_addr(&self) -> SocketAddr {
        self.bind_addr
    }

    /// Returns current statistics
    pub fn stats(&self) -> IngestionStats {
        self.stats
    }

    /// Process a single incoming packet
    ///
    /// # Returns
    /// * `Some((DroneTelemetry, SocketAddr))` - Successfully parsed packet
    /// * `None` - No data available (non-blocking)
    /// * `Err(ParseError)` - Parsing failed
    pub fn recv_one(&mut self) -> Option<Result<(DroneTelemetry, SocketAddr), ParseError>> {
        match self.socket.recv_from(&mut self.buffer) {
            Ok((n, addr)) => {
                self.stats.packets_received += 1;

                // Parse the packet
                let result = crate::protocol::parse_packet(&self.buffer[..n]);

                match &result {
                    Ok(frame) => {
                        self.stats.packets_valid += 1;
                        Some(Ok((*frame, addr)))
                    }
                    Err(e) => {
                        self.stats.packets_invalid += 1;
                        match e {
                            ParseError::InvalidSize => self.stats.errors_size += 1,
                            ParseError::InvalidMagic => self.stats.errors_magic += 1,
                            ParseError::CorruptedChecksum => self.stats.errors_checksum += 1,
                        }
                        Some(Err(*e))
                    }
                }
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                // No data available (non-blocking)
                None
            }
            Err(e) => {
                eprintln!("Socket error: {}", e);
                None
            }
        }
    }

    /// Run the ingestion loop with a provided handler
    /// This is the main event loop - runs until explicitly stopped
    ///
    /// # Arguments
    /// * `handler` - Handler to process frames
    /// * `max_iterations` - Optional limit on iterations (for testing)
    pub fn run<H: TelemetryHandler>(
        &mut self,
        handler: &mut H,
        max_iterations: Option<u64>,
    ) {
        let start_time = std::time::Instant::now();
        let mut iterations = 0u64;

        loop {
            // Check iteration limit
            if let Some(max) = max_iterations {
                if iterations >= max {
                    break;
                }
            }

            // Create PollFd inside the loop to avoid borrow conflicts
            let mut poll_fds = [PollFd::new(&self.socket, PollFlags::POLLIN)];

            // Wait for socket readiness
            match poll(&mut poll_fds, POLL_TIMEOUT_MS) {
                Ok(n) if n > 0 => {
                    // Socket has data ready - process all available packets
                    while let Some(result) = self.recv_one() {
                        match result {
                            Ok((frame, _addr)) => {
                                handler.handle_frame(frame);
                            }
                            Err(e) => {
                                handler.handle_error(
                                    self.socket.local_addr().unwrap_or_else(|_| {
                                        "0.0.0.0:0".parse().unwrap()
                                    }),
                                    e,
                                );
                            }
                        }
                    }
                }
                Ok(_) => {
                    // Timeout - no data available
                }
                Err(e) => {
                    eprintln!("Poll error: {}", e);
                    break;
                }
            }

            iterations += 1;
        }

        let elapsed = start_time.elapsed();
        let rate = if elapsed.as_secs_f64() > 0.0 {
            self.stats.packets_received as f64 / elapsed.as_secs_f64()
        } else {
            0.0
        };

        println!(
            "[Ingestion] Stopped after {} iterations in {:.2}s",
            iterations,
            elapsed.as_secs_f64()
        );
        println!(
            "[Stats] Packets: {} total, {} valid, {} invalid",
            self.stats.packets_received, self.stats.packets_valid, self.stats.packets_invalid
        );
        println!("[Stats] Rate: {:.0} packets/sec", rate);
        println!(
            "[Stats] Errors: {} size, {} magic, {} checksum",
            self.stats.errors_size, self.stats.errors_magic, self.stats.errors_checksum
        );
        println!(
            "[Stats] Error rate: {:.2}%",
            self.stats.error_rate() * 100.0
        );
    }

    /// Reset statistics
    pub fn reset_stats(&mut self) {
        self.stats.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_engine_creation() {
        let engine = IngestionEngine::new("127.0.0.1:0");
        assert!(engine.is_ok());
    }

    #[test]
    fn test_engine_binding() {
        let engine = IngestionEngine::new("127.0.0.1:9999");
        assert!(engine.is_ok());
        let eng = engine.unwrap();
        let addr = eng.bound_addr();
        assert_eq!(addr.port(), 9999);
    }

    #[test]
    fn test_stats_default() {
        let stats = IngestionStats::default();
        assert_eq!(stats.packets_received, 0);
        assert_eq!(stats.packets_valid, 0);
        assert_eq!(stats.error_rate(), 0.0);
    }

    #[test]
    fn test_stats_error_rate() {
        let mut stats = IngestionStats::default();
        stats.packets_received = 100;
        stats.packets_invalid = 10;
        assert!((stats.error_rate() - 0.1).abs() < 0.001);
    }

    #[test]
    fn test_recv_no_data_nonblocking() {
        let mut engine = IngestionEngine::new("127.0.0.1:0").unwrap();
        // Should return None immediately (no data)
        let result = engine.recv_one();
        assert!(result.is_none());
    }
}
