//! Global hold-to-talk key listener: one listener for every key Murmur uses.

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HotkeyEvent {
    Pressed,
    Released,
    /// Another key was pressed while holding (e.g. Fn+F5, Fn+Arrow); not a dictation.
    Cancelled,
    /// Esc was pressed (whether or not the hotkey is held).
    Escape,
}

/// Which of Murmur's keys an event is about.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hotkey {
    Dictation,
    Assistant,
}

/// Returns true when the event was consumed (lets the listener swallow Esc while recording).
/// Esc is offered to each key in turn until one takes it.
pub type Handler = Box<dyn Fn(Hotkey, HotkeyEvent) -> bool + Send + 'static>;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::start;

#[cfg(not(target_os = "macos"))]
mod other;
#[cfg(not(target_os = "macos"))]
pub use other::start;
