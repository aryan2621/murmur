//! Windows (and Linux) listener using a low-level keyboard hook via `rdev`.

use super::{Handler, HotkeyEvent};
use anyhow::{anyhow, Result};
use rdev::{listen, EventType, Key};

fn key_spec(name: &str) -> Result<Key> {
    Ok(match name {
        "right_ctrl" | "right_control" => Key::ControlRight,
        "right_alt" => Key::AltGr,
        "caps_lock" => Key::CapsLock,
        other => return Err(anyhow!("Unsupported hotkey '{other}'")),
    })
}

pub fn start(key: &str, handler: Handler) -> Result<()> {
    let target = key_spec(key)?;
    std::thread::Builder::new().name("murmur-hotkey".into()).spawn(move || {
        let mut held = false;
        let result = listen(move |event| match event.event_type {
            // Key auto-repeat sends repeated presses while held; only the first counts.
            EventType::KeyPress(k) if k == target => {
                if !held {
                    held = true;
                    let _ = handler(HotkeyEvent::Pressed);
                }
            }
            EventType::KeyRelease(k) if k == target => {
                if held {
                    held = false;
                    let _ = handler(HotkeyEvent::Released);
                }
            }
            EventType::KeyPress(Key::Escape) => {
                let _ = handler(HotkeyEvent::Escape);
            }
            EventType::KeyPress(_) if held => {
                held = false;
                let _ = handler(HotkeyEvent::Cancelled);
            }
            _ => {}
        });
        if let Err(e) = result {
            mlog!("keyboard hook failed: {e:?}");
        }
    })?;
    Ok(())
}
