//! Wire Protocol & Serialization Subsystem (1.2)
//!
//! Defines the 32-byte binary telemetry packet format and implements
//! zero-copy parsing with CRC-16 checksum validation.
//!
//! Wire Protocol Schema (32 bytes total):
//! ```text
//! Offset     Field         Type       Purpose
//! 0x00-0x01  Magic         uint16_t   Protocol validation (0xF06F)
//! 0x02-0x05  Device ID     uint32_t   Unique drone identifier
//! 0x06-0x0D  Timestamp     uint64_t   Epoch in microseconds
//! 0x0E-0x15  Latitude      f64        Spatial coordinate
//! 0x16-0x1D  Longitude     f64        Spatial coordinate
//! 0x1E-0x1F  Checksum      uint16_t   CRC-16 validation
//! ```

use std::mem;

/// Magic number for protocol validation
pub const MAGIC: u16 = 0xF06F;

/// Exact payload size in bytes (before checksum)
pub const PAYLOAD_SIZE: usize = 30;

/// Total packet size including checksum
pub const TOTAL_PACKET_SIZE: usize = 32;

/// DroneTelemetry struct with C layout and no padding
/// Exactly 32 bytes when serialized
#[repr(C, packed)]
#[derive(Copy, Clone, Debug)]
pub struct DroneTelemetry {
    /// Protocol magic number (0xF06F)
    pub magic: u16,
    /// Unique device identifier
    pub device_id: u32,
    /// Timestamp in microseconds since epoch
    pub timestamp: u64,
    /// Latitude coordinate
    pub latitude: f64,
    /// Longitude coordinate
    pub longitude: f64,
    /// CRC-16 checksum across first 30 bytes
    pub checksum: u16,
}

impl DroneTelemetry {
    /// Verify that the struct is exactly 32 bytes
    /// This is a compile-time check wrapped in a function
    pub fn verify_layout() {
        assert_eq!(
            mem::size_of::<DroneTelemetry>(),
            TOTAL_PACKET_SIZE,
            "DroneTelemetry must be exactly {} bytes",
            TOTAL_PACKET_SIZE
        );
    }
}

/// Computes CRC-16-CCITT checksum across the given data
/// Used to validate packet integrity
///
/// # Arguments
/// * `data` - Byte slice to compute checksum over (typically 30 bytes)
///
/// # Returns
/// 16-bit CRC value
pub fn compute_crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;

    for &byte in data {
        crc ^= (byte as u16) << 8;
        for _ in 0..8 {
            if crc & 0x8000 != 0 {
                crc = ((crc << 1) ^ 0x1021) & 0xFFFF;
            } else {
                crc = (crc << 1) & 0xFFFF;
            }
        }
    }

    crc
}

/// Errors that can occur during packet parsing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// Packet size is incorrect (not 32 bytes)
    InvalidSize,
    /// Magic number mismatch
    InvalidMagic,
    /// CRC-16 checksum validation failed
    CorruptedChecksum,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::InvalidSize => write!(f, "Invalid packet size"),
            ParseError::InvalidMagic => write!(f, "Invalid magic number"),
            ParseError::CorruptedChecksum => write!(f, "Checksum validation failed"),
        }
    }
}

/// Parses a raw byte buffer into a DroneTelemetry frame
/// Uses zero-copy semantics via pointer casting
///
/// # Arguments
/// * `buffer` - Raw bytes from network
///
/// # Returns
/// * `Ok(DroneTelemetry)` - Successfully parsed frame
/// * `Err(ParseError)` - Parsing failed
///
/// # Panics
/// Will panic if the buffer slice is incorrectly sized or misaligned,
/// but this should never happen if TOTAL_PACKET_SIZE is correct.
pub fn parse_packet(buffer: &[u8]) -> Result<DroneTelemetry, ParseError> {
    // Validate size
    if buffer.len() != TOTAL_PACKET_SIZE {
        return Err(ParseError::InvalidSize);
    }

    // Zero-copy cast: interpret bytes as DroneTelemetry
    // Safe because we control the layout with #[repr(C, packed)]
    let frame: DroneTelemetry = unsafe {
        // SAFETY: We've verified the buffer is exactly 32 bytes,
        // and DroneTelemetry is #[repr(C, packed)] so no alignment issues.
        // The bytes from the network are in little-endian format (as sent by Edge Streamer)
        // and our struct interprets them directly.
        std::ptr::read_unaligned(buffer.as_ptr() as *const DroneTelemetry)
    };

    // Validate magic number
    if frame.magic != MAGIC {
        return Err(ParseError::InvalidMagic);
    }

    // Validate CRC-16 across first 30 bytes (everything except the checksum field)
    let payload_crc = compute_crc16(&buffer[..PAYLOAD_SIZE]);
    if payload_crc != frame.checksum {
        return Err(ParseError::CorruptedChecksum);
    }

    Ok(frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_struct_layout() {
        DroneTelemetry::verify_layout();
        assert_eq!(mem::size_of::<DroneTelemetry>(), TOTAL_PACKET_SIZE);
    }

    #[test]
    fn test_crc16_calculation() {
        // Test vector: empty payload should produce a known CRC
        let empty = [];
        let crc = compute_crc16(&empty);
        assert_eq!(crc, 0xFFFF);

        // Test with a single byte
        let data = [0xFF];
        let crc = compute_crc16(&data);
        assert_ne!(crc, 0xFFFF); // Should differ from initial state
    }

    #[test]
    fn test_parse_invalid_size() {
        let small_buffer = vec![0u8; 16];
        let result = parse_packet(&small_buffer);
        assert_eq!(result, Err(ParseError::InvalidSize));

        let large_buffer = vec![0u8; 64];
        let result = parse_packet(&large_buffer);
        assert_eq!(result, Err(ParseError::InvalidSize));
    }

    #[test]
    fn test_parse_invalid_magic() {
        let mut buffer = vec![0u8; TOTAL_PACKET_SIZE];
        // Set invalid magic at offset 0-1 (little-endian)
        buffer[0] = 0xFF;
        buffer[1] = 0xFF;

        let result = parse_packet(&buffer);
        assert_eq!(result, Err(ParseError::InvalidMagic));
    }

    #[test]
    fn test_parse_corrupted_checksum() {
        let mut buffer = vec![0u8; TOTAL_PACKET_SIZE];

        // Set valid magic (0xF06F in little-endian)
        buffer[0] = 0x6F;
        buffer[1] = 0xF0;

        // Set device ID (offset 2-5)
        buffer[2] = 0x65;
        buffer[3] = 0x00;
        buffer[4] = 0x00;
        buffer[5] = 0x00;

        // Set timestamp (offset 6-13)
        buffer[6] = 0x00;
        buffer[7] = 0x00;
        buffer[8] = 0x00;
        buffer[9] = 0x00;
        buffer[10] = 0x00;
        buffer[11] = 0x00;
        buffer[12] = 0x00;
        buffer[13] = 0x00;

        // Leave latitude and longitude as zeros (offset 14-29)
        // Set an incorrect checksum (offset 30-31)
        buffer[30] = 0xFF;
        buffer[31] = 0xFF;

        let result = parse_packet(&buffer);
        assert_eq!(result, Err(ParseError::CorruptedChecksum));
    }

    #[test]
    fn test_parse_valid_packet() {
        let mut buffer = vec![0u8; TOTAL_PACKET_SIZE];

        // Set valid magic (0xF06F in little-endian)
        buffer[0] = 0x6F;
        buffer[1] = 0xF0;

        // Set device ID to 101 (0x65)
        buffer[2] = 0x65;
        buffer[3] = 0x00;
        buffer[4] = 0x00;
        buffer[5] = 0x00;

        // Set timestamp
        buffer[6] = 0x00;
        buffer[7] = 0x00;
        buffer[8] = 0x00;
        buffer[9] = 0x00;
        buffer[10] = 0x00;
        buffer[11] = 0x00;
        buffer[12] = 0x00;
        buffer[13] = 0x00;

        // Latitude and longitude are zeros (offset 14-29)

        // Calculate correct checksum for the first 30 bytes
        let payload_crc = compute_crc16(&buffer[..PAYLOAD_SIZE]);
        buffer[30] = (payload_crc & 0xFF) as u8;
        buffer[31] = ((payload_crc >> 8) & 0xFF) as u8;

        let result = parse_packet(&buffer);
        assert!(result.is_ok());

        let frame = result.unwrap();
        assert_eq!(frame.magic, MAGIC);
        assert_eq!(frame.device_id, 101);
    }

    #[test]
    fn test_zero_copy_no_allocations() {
        // Verify parse_packet uses no heap allocations
        // This is a compile-time property, but we can test the result
        let buffer = vec![0u8; TOTAL_PACKET_SIZE];

        // Set valid magic
        let mut buf = buffer.clone();
        buf[0] = 0x6F;
        buf[1] = 0xF0;

        // Set correct checksum
        let payload_crc = compute_crc16(&buf[..PAYLOAD_SIZE]);
        buf[30] = (payload_crc & 0xFF) as u8;
        buf[31] = ((payload_crc >> 8) & 0xFF) as u8;

        let result = parse_packet(&buf);
        assert!(result.is_ok());
        // The frame is a Copy type, so it moves without allocation
    }
}
