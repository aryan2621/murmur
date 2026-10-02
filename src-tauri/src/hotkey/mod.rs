//! Global hold-to-talk key listener.

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HotkeyEvent {
    Pressed,
    Released,
    /// Another key was pressed while holding (e.g. Fn+F5, Fn+Arrow); not a dictation.
    Cancelled,
    /// Esc was pressed (whether or not the hotkey is held).
    Escape,
}

/// Returns true when the event was consumed (lets the listener swallow Esc while recording).
pub type Handler = Box<dyn Fn(HotkeyEvent) -> bool + Send + 'static>;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::start;

#[cfg(not(target_os = "macos"))]
mod other;
#[cfg(not(target_os = "macos"))]
pub use other::start;
