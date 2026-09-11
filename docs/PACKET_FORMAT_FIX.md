# Packet Size Fix - The Issue & Solution

## 🔴 Problem: "Invalid packet size" Error

The daemon was rejecting all packets from the edge streamer with `ParseError::InvalidSize` because the Python code was sending **30-byte packets** instead of the required **32-byte packets**.

---

## 🔍 Root Cause

### The Rust Daemon Expects:
```
Offset     Field         Type      Size    Total
0x00-0x01  Magic         u16       2       2
0x02-0x05  Device ID     u32       4       6
0x06-0x0D  Timestamp     u64       8       14
0x0E-0x15  Latitude      f64       8       22
0x16-0x1D  Longitude     f64       8       30
0x1E-0x1F  Checksum      u16       2       32 ✓
```

### What the Python Code Was Sending:
```python
payload = struct.pack('<HIQfffH', *frame)
```

Breaking this down:
- H = u16 (magic) = 2 bytes
- I = u32 (device_id) = 4 bytes
- Q = u64 (timestamp) = 8 bytes
- f = float (latitude) = 4 bytes ← WRONG! Should be 'd' (double = 8)
- f = float (longitude) = 4 bytes ← WRONG! Should be 'd' (double = 8)
- f = float (speed) = 4 bytes ← WRONG! Shouldn't be here!
- H = u16 (battery/checksum) = 2 bytes

**Total sent**: 2+4+8+4+4+4+2 = **28 bytes payload + 2 bytes checksum = 30 bytes total**

**Expected**: 2+4+8+8+8+2 = **32 bytes total**

---

## ✅ The Fix

### Changed the struct.pack() format:

```python
# BEFORE (WRONG):
payload = struct.pack('<HIQfffH', *frame)
# → Sends: magic(2) + device_id(4) + timestamp(8) + latitude(4) + longitude(4) + speed(4) + battery(2)
# → Size: 28 bytes

# AFTER (CORRECT):
payload = struct.pack('<HIQdd', frame.magic, frame.DroneID, frame.timestamp, 
                     frame.latitude, frame.longitude)
# → Sends: magic(2) + device_id(4) + timestamp(8) + latitude(8) + longitude(8)
# → Size: 30 bytes
# Then add 2 bytes for CRC = 32 bytes total ✓
```

### Changed the TelemetryFrame struct:

```python
# BEFORE (WRONG):
class TelemetryFrame(NamedTuple):
    magic: int
    DroneID: int
    timestamp: int
    latitude: float
    longitude: float
    speed: float        # ← WRONG! Not part of wire protocol
    battery: int        # ← WRONG! Replaced by checksum

# AFTER (CORRECT):
class TelemetryFrame(NamedTuple):
    magic: int
    DroneID: int
    timestamp: int
    latitude: float
    longitude: float
    checksum: int       # ← Placeholder, calculated during packing
```

### Updated packTelemetry():

```python
# BEFORE (WRONG):
def packTelemetry(frame: TelemetryFrame) -> bytes:
    payload = struct.pack('<HIQfffH', *frame)  # 28 bytes
    crc = DroneState.generateChecksum(payload)
    return payload + struct.pack('<H', crc)    # + 2 bytes = 30 total

# AFTER (CORRECT):
def packTelemetry(frame: TelemetryFrame) -> bytes:
    # Pack first 30 bytes (everything except checksum field)
    payload = struct.pack('<HIQdd', frame.magic, frame.DroneID, frame.timestamp, 
                         frame.latitude, frame.longitude)
    
    # Calculate CRC-16 over the 30-byte payload
    crc = DroneState.generateChecksum(payload)
    
    # Return payload + checksum (2 bytes) = 32 bytes total
    return payload + struct.pack('<H', crc)    # 30 + 2 = 32 total ✓
```

---

## 📊 Packet Format Comparison

### BEFORE (30 bytes - WRONG):
```
Byte Layout:
0x00 0x01 | 0x02 0x03 0x04 0x05 | 0x06-0x0D | 0x0E-0x11 | 0x12-0x15 | 0x16-0x19 | 0x1A 0x1B
magic     | device_id             | timestamp  | lat(f)    | lon(f)    | speed(f)  | battery
2 bytes   | 4 bytes               | 8 bytes    | 4 bytes   | 4 bytes   | 4 bytes   | 2 bytes
          |                       |            |           |           |           |
          └─────────────────────┬─────────────┴───────────┴───────────┴───────────┘
                                │ CRC calculated over this (28 bytes)
                                └─ CRC appended (2 bytes)

Total: 28 + 2 = 30 bytes ❌
```

### AFTER (32 bytes - CORRECT):
```
Byte Layout:
0x00 0x01 | 0x02 0x03 0x04 0x05 | 0x06-0x0D | 0x0E-0x15   | 0x16-0x1D   | 0x1E 0x1F
magic     | device_id             | timestamp  | lat(d)      | lon(d)      | checksum
2 bytes   | 4 bytes               | 8 bytes    | 8 bytes     | 8 bytes     | 2 bytes
          |                       |            |             |             |
          └─────────────────────┬─────────────┴─────────────┴─────────────┘
                                │ CRC calculated over this (30 bytes)
                                └─ CRC appended (2 bytes)

Total: 30 + 2 = 32 bytes ✓
```

---

## 🧪 Verification

Run this Python test to verify packet size:

```python
import struct

magic = 0xF06F
device_id = 101
timestamp = 1694328456123456
latitude = 13.0827
longitude = 80.2707

# Pack exactly as daemon expects
payload = struct.pack('<HIQdd', magic, device_id, timestamp, latitude, longitude)
checksum = struct.pack('<H', 0)
packet = payload + checksum

print(f"Payload: {len(payload)} bytes")
print(f"Checksum: {len(checksum)} bytes")
print(f"Total: {len(packet)} bytes")

assert len(packet) == 32, f"Expected 32, got {len(packet)}"
print("✓ Packet format is CORRECT!")
```

Expected output:
```
Payload: 30 bytes
Checksum: 2 bytes
Total: 32 bytes
✓ Packet format is CORRECT!
```

---

## 🚀 Next Steps

### 1. Download the Fixed Edge Streamer
```
edge_streamer_redesigned.py (UPDATED)
```

### 2. Test with Daemon

**Terminal 1:**
```bash
cd ~/vakhd
cargo run --release
# Expected: Listening for telemetry packets...
```

**Terminal 2:**
```bash
python3 edge_streamer_redesigned.py
# Expected: [Frame #1000], [Frame #2000], etc. (NO errors)
```

### 3. Verify No Errors
The daemon should now show:
```
[Frame #1000] Device=101, Lat=13.0821, Lon=80.2705, Ts=...
[Frame #2000] Device=102, Lat=13.0910, Lon=80.2745, Ts=...
...
Valid packets: 240000
Invalid packets: 0
Error rate: 0.000%
```

---

## 💡 Key Learnings

### Struct Packing Format Codes
- `H` = unsigned short (2 bytes)
- `I` = unsigned int (4 bytes)
- `Q` = unsigned long long (8 bytes)
- `f` = float (4 bytes) ← For coordinates, need 8-byte precision
- `d` = double (8 bytes) ← Correct for lat/lon

### Binary Protocol Rules
1. **Always match the spec exactly** - Every byte counts
2. **Use appropriate precision** - GPS coordinates need 64-bit precision (double), not 32-bit (float)
3. **Calculate checksums correctly** - CRC is over payload BEFORE checksum field
4. **Test with actual bytes** - Verify packet size with struct.calcsize()

---

## 📋 Summary

| Aspect | Before | After | Status |
|--------|--------|-------|--------|
| **Packet Size** | 30 bytes | 32 bytes | ✅ Fixed |
| **Latitude** | float (4B) | double (8B) | ✅ Fixed |
| **Longitude** | float (4B) | double (8B) | ✅ Fixed |
| **Extra Fields** | speed, battery | removed | ✅ Removed |
| **Invalid Size Errors** | Always | Never | ✅ Solved |

---

## Files Updated

✅ `edge_streamer_redesigned.py` - Fixed packet format

Download from `/mnt/user-data/outputs/` and your daemon will work perfectly!

