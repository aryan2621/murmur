//! Turns the Mac's sound down while the assistant listens, like Siri does. Otherwise a song or
//! video playing on the Mac (often one the assistant just started) reaches the mic, sounds like
//! you still talking, and listening never ends. Changes run in order on their own thread, so a
//! quick listen-then-stop can't leave the sound turned down.

use std::process::Command;
use std::sync::mpsc;

/// While listening, the volume is cut to this share of what it was.
const DUCKED_SHARE: i64 = 4;
/// At or below this volume there's nothing worth turning down.
const MIN_VOLUME: i64 = 12;

enum Cmd {
    Duck,
    Restore(mpsc::Sender<()>),
    Settle(mpsc::Sender<()>),
}

pub struct Ducker {
    tx: mpsc::Sender<Cmd>,
}

impl Ducker {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<Cmd>();
        std::thread::Builder::new()
            .name("murmur-duck".into())
            .spawn(move || {
                // The volume to put back, while it's turned down.
                let mut saved: Option<i64> = None;
                for cmd in rx {
                    match cmd {
                        Cmd::Duck if saved.is_none() => saved = duck(),
                        Cmd::Duck => {}
                        Cmd::Restore(done) => {
                            if let Some(v) = saved.take() {
                                if let Err(e) = osascript(&format!("set volume output volume {v}")) {
                                    mlog!("could not turn the sound back up: {e}");
                                }
                            }
                            let _ = done.send(());
                        }
                        Cmd::Settle(done) => {
                            let _ = done.send(());
                        }
                    }
                }
            })
            .expect("spawn duck thread");
        Self { tx }
    }

    /// Turns the sound down (returns at once).
    pub fn duck(&self) {
        let _ = self.tx.send(Cmd::Duck);
    }

    /// Turns the sound down and returns once it is, so the first moments of listening don't hear
    /// the song at full volume and take it for you starting to speak.
    pub fn duck_now(&self) {
        self.duck();
        self.wait(Cmd::Settle);
    }

    /// Puts the sound back as it was. Returns once it has, so an action that sets the volume
    /// itself can't be undone by this coming later.
    pub fn restore(&self) {
        self.wait(Cmd::Restore);
    }

    fn wait(&self, cmd: fn(mpsc::Sender<()>) -> Cmd) {
        let (done, wait) = mpsc::channel();
        if self.tx.send(cmd(done)).is_ok() {
            let _ = wait.recv();
        }
    }
}

/// Turns the sound down unless it's muted or already quiet. Returns the volume to put back.
fn duck() -> Option<i64> {
    let script = format!(
        "set s to get volume settings\n\
         set v to output volume of s\n\
         if output muted of s or v is missing value or v ≤ {MIN_VOLUME} then return -1\n\
         set volume output volume (v div {DUCKED_SHARE})\n\
         return v"
    );
    match osascript(&script).map(|out| out.trim().parse::<i64>()) {
        Ok(Ok(v)) if v >= 0 => {
            mlog!("sound turned down while listening ({v} → {})", v / DUCKED_SHARE);
            Some(v)
        }
        Ok(_) => None,
        Err(e) => {
            mlog!("could not turn the sound down: {e}");
            None
        }
    }
}

fn osascript(script: &str) -> anyhow::Result<String> {
    let out = Command::new("osascript").arg("-e").arg(script).output()?;
    if !out.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
