// MAVLink Protocol Module
// Handles MAVLink v1/v2 frame parsing, serialization, handshake, and handler thread.
// Uses the `mavlink` crate for message definitions (ardupilotmega dialect).

pub mod codec;
pub mod handler;
pub mod handshake;
pub mod mission;
pub mod parser;

pub use handler::{MavlinkHandle, MavlinkCommand};
pub use handshake::perform_handshake;
pub use mission::ArduWaypoint;
