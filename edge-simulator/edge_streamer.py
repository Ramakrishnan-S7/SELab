from typing import NamedTuple
import random
import time
import struct
import socket
import threading
from dataclasses import dataclass
import logging

# Configure logging
logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(name)s - %(levelname)s - %(message)s'
)

class TelemetryFrame(NamedTuple):
    magic: int
    DroneID: int
    timestamp: int
    latitude: float
    longitude: float
    checksum: int


@dataclass
class DroneConfig:
    """Configuration for a drone instance"""
    drone_id: int
    latitude: float
    longitude: float
    target_host: str
    target_port: int


class DroneState:
    """Manages state and telemetry generation for a single drone"""
    
    def __init__(self, config: DroneConfig):
        self.logger = logging.getLogger(f"Drone-{config.drone_id}")
        self.config = config
        
        # Drone state
        self.magic = 0xF06F
        self.drone_id = config.drone_id
        self.battery = 1000
        self.latitude = config.latitude
        self.longitude = config.longitude
        self.speed = 15.0
        self.timestamp = int(time.time() * 1000000)
        
        # Socket - will be created per drone
        self.socket = None
        self.running = False

    @staticmethod
    def generateChecksum(data: bytes) -> int:
        """CRC-16-CCITT checksum calculation"""
        crc = 0xFFFF
        for byte in data:
            crc ^= (byte << 8)
            for _ in range(8):
                if crc & 0x8000:
                    crc = ((crc << 1) ^ 0x1021) & 0xFFFF
                else:
                    crc = (crc << 1) & 0xFFFF
        return crc

    def generate(self) -> TelemetryFrame:
        """Generate next telemetry frame with updated drone state"""
        self.timestamp = int(time.time() * 1000000)
        
        if self.battery > 0:
            # Update position with random walk
            self.latitude += random.uniform(-0.0002, 0.0003)
            self.longitude += random.uniform(-0.0003, 0.0002)
            
            # Update speed (tracked internally but not sent)
            self.speed = max(5.0, min(25.0, self.speed + random.uniform(-0.5, 0.5)))
            
            # Battery drain with 20% probability per frame
            if random.random() < 0.2:
                self.battery -= 1
        else:
            # Drone powered down
            self.speed = 0

        # Return frame with all required fields
        # Checksum will be calculated in packTelemetry, so use 0 as placeholder
        return TelemetryFrame(
            magic=self.magic,
            DroneID=self.drone_id,
            timestamp=self.timestamp,
            latitude=self.latitude,
            longitude=self.longitude,
            checksum=0  # Placeholder, will be calculated
        )

    @staticmethod
    def packTelemetry(frame: TelemetryFrame) -> bytes:
        """Pack telemetry frame into binary format with CRC"""
        # Pack first 30 bytes (everything except checksum field)
        # Format: magic(H=2) + device_id(I=4) + timestamp(Q=8) + latitude(d=8) + longitude(d=8)
        payload = struct.pack('<HIQdd', frame.magic, frame.DroneID, frame.timestamp, 
                             frame.latitude, frame.longitude)
        
        # Calculate CRC-16 over the 30-byte payload
        crc = DroneState.generateChecksum(payload)
        
        # Return payload + checksum (2 bytes) = 32 bytes total
        return payload + struct.pack('<H', crc)

    def create_socket(self) -> bool:
        """Create and bind UDP socket for this drone"""
        try:
            self.socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            self.socket.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            
            # Enable non-blocking mode for graceful shutdown
            self.socket.setblocking(True)
            
            self.logger.info(
                f"Socket created. Target: {self.config.target_host}:{self.config.target_port}"
            )
            return True
        except OSError as e:
            self.logger.error(f"Failed to create socket: {e}")
            return False

    def send_telemetry(self, frame: TelemetryFrame) -> bool:
        """Send a single telemetry frame to the daemon"""
        try:
            packed_data = self.packTelemetry(frame)
            
            self.socket.sendto(
                packed_data,
                (self.config.target_host, self.config.target_port)
            )
            
            self.logger.debug(
                f"Sent: Lat={frame.latitude:.4f}, Lon={frame.longitude:.4f}, "
                f"Speed={self.speed:.2f}, Battery={self.battery}"
            )
            return True
            
        except OSError as e:
            self.logger.error(f"Failed to send telemetry: {e}")
            return False

    def close_socket(self):
        """Close the socket"""
        if self.socket:
            try:
                self.socket.close()
                self.logger.info("Socket closed")
            except OSError as e:
                self.logger.error(f"Error closing socket: {e}")
            finally:
                self.socket = None

    def run(self, tick_rate: int = 10, duration: int = None):
        """
        Run the drone telemetry streamer.
        
        Args:
            tick_rate: Messages per second
            duration: Optional duration in seconds before stopping
        """
        if not self.create_socket():
            return
        
        self.running = True
        frame_count = 0
        start_time = time.time()
        tick_interval = 1.0 / tick_rate

        try:
            self.logger.info(f"Started streaming at {tick_rate} Hz")
            
            while self.running:
                # Check duration limit
                if duration and (time.time() - start_time) > duration:
                    self.logger.info(f"Duration limit reached ({duration}s)")
                    break

                frame = self.generate()
                
                if self.send_telemetry(frame):
                    frame_count += 1
                else:
                    # Log but continue trying to send
                    pass

                # Maintain tick rate
                time.sleep(tick_interval)

        except KeyboardInterrupt:
            self.logger.info("Interrupted by user")
        except Exception as e:
            self.logger.error(f"Unexpected error: {e}")
        finally:
            self.close_socket()
            self.logger.info(f"Stopped after {frame_count} frames")


def run_drone_streamer(config: DroneConfig, tick_rate: int = 10, duration: int = None):
    """
    Thread target function for running a drone.
    
    Args:
        config: DroneConfig with drone parameters
        tick_rate: Messages per second
        duration: Optional duration in seconds
    """
    drone = DroneState(config)
    drone.run(tick_rate=tick_rate, duration=duration)


def main():
    """Main entry point - starts multiple drones with their own sockets"""
    
    # Configuration
    DAEMON_HOST = "127.0.0.1"
    DAEMON_PORT = 8081  # Updated from 8080 (port in use)
    TICK_RATE = 10  # Messages per second
    DURATION = 60   # Run for 60 seconds (set to None for infinite)
    
    # Define drone configurations with different starting locations
    drone_configs = [
        DroneConfig(
            drone_id=101,
            latitude=13.0827,      # Chennai
            longitude=80.2707,
            target_host=DAEMON_HOST,
            target_port=DAEMON_PORT
        ),
        DroneConfig(
            drone_id=102,
            latitude=13.0900,
            longitude=80.2750,
            target_host=DAEMON_HOST,
            target_port=DAEMON_PORT
        ),
        DroneConfig(
            drone_id=103,
            latitude=13.0750,
            longitude=80.2650,
            target_host=DAEMON_HOST,
            target_port=DAEMON_PORT
        ),
        DroneConfig(
            drone_id=104,
            latitude=13.0800,
            longitude=80.2800,
            target_host=DAEMON_HOST,
            target_port=DAEMON_PORT
        ),
    ]
    
    # Create and start threads for each drone
    threads = []
    
    print(f"\n{'='*70}")
    print(f"Edge Streamer - Multi-Drone Telemetry Simulator")
    print(f"{'='*70}")
    print(f"Daemon Target: {DAEMON_HOST}:{DAEMON_PORT}")
    print(f"Tick Rate: {TICK_RATE} Hz per drone")
    print(f"Duration: {DURATION}s" if DURATION else "Duration: Infinite")
    print(f"Number of Drones: {len(drone_configs)}")
    print(f"{'='*70}\n")
    
    for config in drone_configs:
        thread = threading.Thread(
            target=run_drone_streamer,
            args=(config, TICK_RATE, DURATION),
            name=f"Drone-{config.drone_id}",
            daemon=False
        )
        threads.append(thread)
        thread.start()
        print(f"[✓] Started Drone {config.drone_id}")
    
    # Wait for all threads to complete
    try:
        for thread in threads:
            thread.join()
        print(f"\n{'='*70}")
        print("All drones have stopped streaming")
        print(f"{'='*70}\n")
    except KeyboardInterrupt:
        print(f"\n{'='*70}")
        print("Shutting down all drones...")
        print(f"{'='*70}\n")
        for thread in threads:
            thread.join(timeout=2)


if __name__ == "__main__":
    main()
