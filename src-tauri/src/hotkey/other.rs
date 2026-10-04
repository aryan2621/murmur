//! Windows (and Linux) listener using a low-level keyboard hook via `rdev`.

use super::{Handler, Hotkey, HotkeyEvent};
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

pub fn start(keys: &[(String, Hotkey)], handler: Handler) -> Result<()> {
    let keys = keys.iter().map(|(name, id)| key_spec(name).map(|k| (k, *id))).collect::<Result<Vec<_>>>()?;
    std::thread::Builder::new().name("murmur-hotkey".into()).spawn(move || {
        let mut held: Vec<Hotkey> = vec![];
        let result = listen(move |event| match event.event_type {
            EventType::KeyPress(Key::Escape) => {
                for (_, id) in &keys {
                    if handler(*id, HotkeyEvent::Escape) {
                        break;
                    }
                }
            }
            // Key auto-repeat sends repeated presses while held; only the first counts.
            EventType::KeyPress(k) => match keys.iter().find(|(key, _)| *key == k) {
                Some((_, id)) if !held.contains(id) => {
                    held.push(*id);
                    let _ = handler(*id, HotkeyEvent::Pressed);
                }
                Some(_) => {}
                None => {
                    for id in held.drain(..) {
                        let _ = handler(id, HotkeyEvent::Cancelled);
                    }
                }
            },
            EventType::KeyRelease(k) => {
                if let Some((_, id)) = keys.iter().find(|(key, _)| *key == k) {
                    if let Some(i) = held.iter().position(|h| h == id) {
                        held.remove(i);
                        let _ = handler(*id, HotkeyEvent::Released);
                    }
                }
            }
            _ => {}
        });
        if let Err(e) = result {
            mlog!("keyboard hook failed: {e:?}");
        }
    })?;
    Ok(())
}
