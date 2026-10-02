//! Timing rules that turn raw hotkey presses into intents: hold-to-talk, ignored taps,
//! and double-tap to lock hands-free recording.

use std::time::Duration;

/// Presses shorter than this are taps, not dictations.
pub const TAP_MAX: Duration = Duration::from_millis(250);
/// A second press this soon after a tap ended makes it a double-tap.
pub const DOUBLE_TAP_GAP: Duration = Duration::from_millis(400);
/// The widget appears only once a press has lasted this long, so taps don't flash it.
pub const SHOW_DELAY: Duration = Duration::from_millis(180);

#[derive(Debug, PartialEq)]
pub enum ReleaseAction {
    /// Held long enough: transcribe what was said.
    Finish,
    /// A lone tap: discard silently.
    DiscardTap,
    /// Second tap of a double-tap: keep recording hands-free until the next press.
    LockHandsFree,
}

/// `gap_before_press` is the time between the previous tap's release and this press.
pub fn on_release(held_for: Duration, gap_before_press: Option<Duration>) -> ReleaseAction {
    if held_for >= TAP_MAX {
        ReleaseAction::Finish
    } else if gap_before_press.is_some_and(|g| g <= DOUBLE_TAP_GAP) {
        ReleaseAction::LockHandsFree
    } else {
        ReleaseAction::DiscardTap
    }
}
