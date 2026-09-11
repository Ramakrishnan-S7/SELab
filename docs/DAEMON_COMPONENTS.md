# Edge-Fog Gateway Daemon - Component Architecture

Based on SRS Document (24BCE5221) by Ramakrishnan Sankarasubramanian

---

## 1. CORE FUNCTIONAL SUBSYSTEMS

### 1.1 Network Ingestion Subsystem (Section 3.1)
**Purpose**: Asynchronous network multiplexing using Linux kernel notification primitives

#### Components:
- **Linux Socket Binding Setup**
  - Bind UDP socket to configurable local port
  - Configure non-blocking mode (`set_nonblocking(true)`)
  - Handle socket creation and error states

- **epoll Reactor Loop**
  - Monitor incoming network sockets for read availability
  - Non-blocking event notification via Linux `epoll` syscalls
  - Zero polling loop overhead
  - Single-threaded or thread-pooled event loop using Reactor pattern
  - Offload raw bytes immediately to prevent kernel socket buffer drops

- **Throughput Management**
  - Clear kernel socket buffer quickly to prevent OS-level packet drops
  - Handle high-frequency concurrent UDP telemetry streams
  - Target: 100,000 packets/second without dropping

---

### 1.2 Wire Protocol & Serialization Subsystem (Section 3.2)
**Purpose**: Deterministic parsing of raw network packets into internal data structures

#### Wire Protocol Schema (32 bytes total):
```
Offset     Field         Type       Purpose
0x00-0x01  Magic         uint16_t   Protocol validation (0xF06F)
0x02-0x05  Device ID     uint32_t   Unique drone identifier
0x06-0x0D  Timestamp     uint64_t   Epoch in microseconds
0x0E-0x15  Latitude      double     Spatial coordinate
0x16-0x1D  Longitude     double     Spatial coordinate
0x1E-0x1F  Checksum      uint16_t   CRC-16 validation
```

#### Components:
- **C-Packed Struct Definition**
  - `DroneTelemetry` struct with `#[repr(C, packed)]`
  - Exactly 32 bytes (strictly enforced)
  - Cross-platform layout symmetry

- **Byte Deserialization & Zero-Copy Casting**
  - Parse raw byte slices into `DroneTelemetry` without heap allocation
  - Zero-copy semantics via pointer casting
  - 1 microsecond per packet latency target
  - No cloning of internal byte data

- **CRC-16 Checksum Validation**
  - Validate magic number before processing
  - Compute CRC-16 across first 30 bytes
  - Verify against packet's trailing checksum bytes
  - Immediately discard invalid/corrupted packets
  - Log telemetry anomalies

- **Protocol Testing**
  - Unit tests for layout assertions
  - Byte casting validation
  - Invalid magic number detection
  - Corrupted checksum handling
  - 100% test pass rate requirement

---

### 1.3 Concurrency & Memory Management Subsystem (Section 3.3)
**Purpose**: Bridge high-speed ingestion with lower-speed analytics and storage workers

#### Components:
- **Lock-Free Bounded Ring Buffer (MPMC/SPMC)**
  - Multi-Producer Multi-Consumer or Single-Producer Multi-Consumer queue
  - Completely lock-free implementation
  - Atomic memory operations only (no mutexes or condition variables)
  - Sequential consistency constraints
  - Bounded virtual memory during startup
  - Zero mutex dependencies

- **Worker Thread Pool**
  - Continuously dequeue entries from atomic buffer
  - Handle downstream processing and filtering
  - Prepare data for transmission
  - Even workload distribution across CPU cores
  - Efficient atomic back-off strategies when queue is empty

- **Concurrency Testing**
  - ThreadSanitizer validation (RUSTFLAGS="-Z sanitizer=thread")
  - Zero data races or deadlocks under peak loads
  - Multi-producer safety stress tests

- **In-Memory State Cache**
  - Track drone state for each device ID
  - Latest position, speed, battery level
  - Updated by worker threads without contention

---

### 1.4 Storage & Backpressure Control Subsystem (Section 3.4)
**Purpose**: Fault-tolerance during cloud connectivity drops via Write-Ahead Log (WAL)

#### Components:
- **Cloud Health-Check Manager**
  - Maintain continuous TCP heartbeat with cloud server
  - Detect network connection failures immediately
  - Toggle daemon state: Normal ↔ Degraded Mode

- **Append-Only Write-Ahead Log (WAL)**
  - Local persistent binary log on disk (`telemetry.wal`)
  - Synchronous writes with fsync/O_SYNC flags
  - Prevent data caching during power failures
  - Bypass OS cache for reliability
  - Single file handle for sequential appends

- **State-Aware Backpressure Buffering**
  - In Degraded Mode: Bypass cloud transmission
  - Route telemetry directly to WAL instead
  - Prevent memory buffer overflow
  - Automated fallback to disk storage

- **WAL Playback & Recovery Engine**
  - Background playback thread upon cloud restoration
  - Read and transmit WAL entries sequentially
  - Purge log entries after successful transmission
  - Maintain Normal Mode operation concurrently
  - Recover from ungraceful terminations

- **Persistence Guarantees**
  - Zero data loss on cloud disconnection
  - Data safely committed to WAL remains uncorrupted
  - Graceful recovery upon link restoration

---

### 1.5 Sync Engine & Batch Accumulation (Section 1.5)
**Purpose**: Aggregate telemetry and synchronize with cloud backend

#### Components:
- **Timed Micro-Batch Accumulator**
  - Deterministic interval-based snapshotting
  - Capture latest drone state cache at fixed intervals (e.g., 5s)
  - Snapshot on clock boundaries without pausing ingestion
  - Minimal lock contention during state copying

- **Cloud TCP Sync Client**
  - Persistent TCP connection to remote cloud server
  - Flush interval snapshots in batch format
  - Format: JSON-encoded aggregated telemetry
  - Detect socket disconnections promptly
  - Handle connection failures gracefully

- **Batch Transmission Protocol**
  - Aggregate multiple drone states per interval
  - Send as single TCP packet to cloud
  - Cloud acknowledges successful reception
  - Coordinate with WAL on network failures

---

### 1.6 Lifecycle & Signal Management (Section 1.6)
**Purpose**: Graceful startup, shutdown, and error recovery

#### Components:
- **Graceful Signal Handling**
  - Intercept SIGINT (Ctrl+C) and SIGTERM signals
  - Atomic shutdown flag across all threads
  - Halt network ingestion loop
  - Flush ring buffer contents to disk/cloud
  - Close all open file descriptors cleanly
  - Exit with status code 0

- **Crash Tolerance & Recovery**
  - Handle ungraceful system terminations (SIGKILL)
  - Data in ring buffers may be lost (in-memory)
  - WAL-committed data remains uncorrupted
  - Automatic recovery on restart

- **Chaos & Fault Injection Testing**
  - Simulate cloud network drops
  - Test corrupt packet stream handling
  - Test sudden process kills
  - Validate memory bounds (50 MB RSS limit)
  - Verify state recovery cleanly

---

## 2. SUPPORTING INFRASTRUCTURE COMPONENTS

### 2.1 Configuration Management
- Command-line interface (CLI)
- Configuration files
- Daemon parameters
- Log settings
- Cloud server address/port
- WAL file path

### 2.2 Logging & Diagnostics
- Per-subsystem logging
- Telemetry anomaly tracking
- Performance metrics (latency, throughput)
- Error/warning messages
- Log file output

### 2.3 Process Management
- systemd integration
- Service lifecycle
- Resource monitoring
- Linux process signals
- Thread scheduling

---

## 3. PERFORMANCE & RESOURCE CONSTRAINTS

### 3.1 Performance Requirements
- **Throughput**: 100,000 packets/second
- **Latency**: ≤ 50µs (network interface to processing queue)
- **Memory**: ≤ 50 MB RSS (Resident Set Size)
- **CPU**: Non-blocking, efficient back-off strategies

### 3.2 Reliability Requirements
- **Crash Tolerance**: Ungraceful termination handling
- **Data Loss**: Zero loss for WAL-committed data
- **Graceful Shutdown**: All queues flushed, exit code 0
- **Network Resilience**: WAL-based fallback during outages

---

## 4. COMPONENT INTERACTION FLOW

```
┌─────────────────┐
│  Edge Streamer  │
│   (UDP Drones)  │
└────────┬────────┘
         │ UDP Packets (32 bytes each)
         │
         ▼
┌─────────────────────────────────────────────────┐
│         FOG GATEWAY DAEMON                       │
│                                                   │
│  ┌──────────────────────────────────────────┐   │
│  │ 1.3 Network Ingestion Subsystem           │   │
│  │  - epoll Reactor Loop                     │   │
│  │  - UDP Socket Binding                     │   │
│  └────────────────┬─────────────────────────┘   │
│                   │ raw bytes                    │
│                   ▼                              │
│  ┌──────────────────────────────────────────┐   │
│  │ 1.2 Wire Protocol & Serialization        │   │
│  │  - Magic validation                      │   │
│  │  - CRC-16 checksum verification          │   │
│  │  - Zero-copy parsing                     │   │
│  └────────────────┬─────────────────────────┘   │
│                   │ DroneTelemetry structs      │
│                   ▼                              │
│  ┌──────────────────────────────────────────┐   │
│  │ 1.3 Concurrency & Memory Management      │   │
│  │  - Lock-free Ring Buffer (MPMC)          │   │
│  │  - Worker Thread Pool                    │   │
│  │  - In-Memory State Cache                 │   │
│  └─────┬──────────────────────────┬─────────┘   │
│        │                          │              │
│        │ Normal Operation         │ Degraded    │
│        ▼                          ▼  Mode       │
│  ┌───────────────────┐    ┌─────────────────┐  │
│  │ 1.5 Sync Engine   │    │ 1.4 WAL Manager │  │
│  │ - Batch Accum.    │    │ - Append Log    │  │
│  │ - TCP Cloud Sync  │    │ - Playback      │  │
│  └─────────┬─────────┘    └────────┬────────┘  │
│            │ JSON Batches         │ Binary WAL  │
└────────────┼─────────────────────┼─────────────┘
             │                     │
             ▼                     ▼
    ┌──────────────────┐   ┌────────────────┐
    │ Cloud Server     │   │ Local Storage  │
    │ (TCP)            │   │ (telemetry.wal)│
    └──────────────────┘   └────────────────┘
```

---

## 5. KEY DESIGN DECISIONS

### 5.1 Linux-First Architecture
- Exclusively GNU/Linux (kernel 5.0+)
- Native Linux APIs (epoll, fsync)
- No web servers or heavyweight frameworks
- Build from first principles

### 5.2 Zero-Copy Philosophy
- Minimize memory allocations
- Parse directly from socket buffers
- Use pointer casting instead of cloning
- Atomic operations without mutexes

### 5.3 Fault Tolerance Strategy
- Write-Ahead Logging (WAL) for persistence
- Heartbeat-based cloud health checks
- Automatic mode switching (Normal ↔ Degraded)
- Sequential WAL playback on recovery

### 5.4 Asynchronous Processing
- Non-blocking epoll-based ingestion
- Lock-free ring buffer coordination
- Concurrent cloud sync and WAL writing
- No ingestion blocking during storage ops

---

## 6. ACCEPTANCE CRITERIA SUMMARY

| Component | Metric | Target |
|-----------|--------|--------|
| Ingestion | Packets/second | 100,000 |
| Protocol Parsing | Latency | 1 µs/packet |
| Ring Buffer | Mutexes | 0 (lock-free) |
| Memory | RSS | ≤ 50 MB |
| CRC Validation | Accuracy | 100% |
| Cloud Sync | Format | JSON |
| WAL | Data Loss | 0 |
| Shutdown | Exit Code | 0 |

