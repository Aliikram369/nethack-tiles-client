//! A live game session, whatever is carrying it.
//!
//! NetHack over SSH on a public server and NetHack running on this machine
//! differ only in how the bytes get to a pseudo-terminal. Everything above --
//! the demultiplexer, the tile overlay, the status bar -- wants the same two
//! things from either: a stream of bytes coming back, and somewhere to send
//! keystrokes and window sizes. That shape lives here so neither transport has
//! to know about the other.

use tokio::sync::mpsc;

/// Something that happened on the session, delivered to the UI.
#[derive(Debug, Clone)]
pub enum SessionEvent {
    /// Raw bytes from the game, to be demultiplexed.
    Data(Vec<u8>),
    /// Human-readable progress or warning for the status bar.
    Status(String),
    /// The session ended. `reason` is `None` for a clean exit.
    Closed { reason: Option<String> },
}

/// Instructions for the task or thread that owns the transport.
#[derive(Debug)]
pub enum Command {
    Data(Vec<u8>),
    Resize { cols: u32, rows: u32 },
    Disconnect,
}

#[derive(Debug, thiserror::Error)]
#[error("the session is no longer connected")]
pub struct Disconnected;

/// A handle to a running session. Dropping it does not end the session; call
/// [`Session::disconnect`].
#[derive(Debug, Clone)]
pub struct Session {
    commands: mpsc::UnboundedSender<Command>,
}

impl Session {
    pub fn new(commands: mpsc::UnboundedSender<Command>) -> Self {
        Session { commands }
    }

    /// Sends keystrokes to the game.
    pub fn write(&self, bytes: Vec<u8>) -> Result<(), Disconnected> {
        self.send(Command::Data(bytes))
    }

    /// Tells the game the terminal was resized.
    pub fn resize(&self, cols: u32, rows: u32) -> Result<(), Disconnected> {
        self.send(Command::Resize { cols, rows })
    }

    pub fn disconnect(&self) -> Result<(), Disconnected> {
        self.send(Command::Disconnect)
    }

    pub fn is_connected(&self) -> bool {
        !self.commands.is_closed()
    }

    fn send(&self, command: Command) -> Result<(), Disconnected> {
        self.commands.send(command).map_err(|_| Disconnected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_whose_transport_has_gone_reports_disconnected() {
        let (tx, rx) = mpsc::unbounded_channel();
        let session = Session::new(tx);
        assert!(session.is_connected());

        drop(rx);

        assert!(!session.is_connected());
        assert!(session.write(b"x".to_vec()).is_err());
    }
}
