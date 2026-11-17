//! Local Discord desktop RPC. Only explicitly enabled status data is sent.
use serde_json::{json, Value};
use std::{
    io::{self, Read, Write},
    sync::mpsc::{self, RecvTimeoutError, Sender},
    time::Duration,
};

#[derive(Clone, PartialEq, Eq)]
pub struct Activity {
    pub application: String,
    pub build: Option<(String, String)>,
}

#[derive(Default)]
pub struct Presence {
    desired: Option<Activity>,
    worker: Option<Sender<Option<Activity>>>,
}

impl Presence {
    pub fn update(&mut self, desired: Option<Activity>) {
        if self.desired == desired {
            return;
        }
        if self.worker.is_none() {
            let (tx, rx) = mpsc::channel();
            self.worker = Some(tx);
            std::thread::spawn(move || {
                let mut desired = None::<Activity>;
                let mut connection = None;
                loop {
                    match rx.recv_timeout(Duration::from_secs(10)) {
                        Ok(next) => {
                            let next = rx.try_iter().last().unwrap_or(next);
                            if desired.as_ref().map(|a| &a.application)
                                != next.as_ref().map(|a| &a.application)
                            {
                                connection = None;
                            }
                            desired = next;
                        }
                        Err(RecvTimeoutError::Disconnected) => break,
                        Err(RecvTimeoutError::Timeout) => {}
                    }
                    if let Some(activity) = &desired {
                        let result = (|| -> io::Result<()> {
                            if connection.is_none() {
                                let mut pipe = connect()?;
                                frame(
                                    &mut pipe,
                                    0,
                                    &json!({"v": 1, "client_id": activity.application}),
                                )?;
                                let (_, ready) = reply(&mut pipe)?;
                                if ready["evt"] != "READY" {
                                    return Err(io::Error::other("Discord RPC handshake failed"));
                                }
                                connection = Some(pipe);
                            }
                            publish(connection.as_mut().unwrap(), activity)
                        })();
                        if result.is_err() {
                            connection = None;
                        }
                    }
                }
            });
        }
        self.desired = desired.clone();
        if let Some(worker) = &self.worker {
            let _ = worker.send(desired);
        }
    }
}

fn frame(stream: &mut impl Write, op: u32, value: &Value) -> io::Result<()> {
    let payload = serde_json::to_vec(value)?;
    let mut packet = Vec::with_capacity(8 + payload.len());
    packet.extend(op.to_le_bytes());
    packet.extend((payload.len() as u32).to_le_bytes());
    packet.extend(payload);
    stream.write_all(&packet)
}

fn reply(stream: &mut Pipe) -> io::Result<(u32, Value)> {
    let mut header = [0; 8];
    read_exact(stream, &mut header)?;
    let opcode = u32::from_le_bytes(header[..4].try_into().unwrap());
    let size = u32::from_le_bytes(header[4..].try_into().unwrap()) as usize;
    if size > 64 * 1024 {
        return Err(io::Error::other("Discord RPC response too large"));
    }
    let mut payload = vec![0; size];
    read_exact(stream, &mut payload)?;
    Ok((opcode, serde_json::from_slice(&payload)?))
}

fn publish(stream: &mut Pipe, activity: &Activity) -> io::Result<()> {
    let status = match &activity.build {
        Some((name, version)) => json!({
            "details": name.chars().take(128).collect::<String>(),
            "state": format!("Minecraft {}", version.chars().take(110).collect::<String>()),
        }),
        None => json!({"details": "Лаунчер открыт"}),
    };
    let nonce = uuid::Uuid::new_v4().to_string();
    frame(
        stream,
        1,
        &json!({
            "cmd": "SET_ACTIVITY",
            "args": {"pid": std::process::id(), "activity": status},
            "nonce": nonce,
        }),
    )?;
    for _ in 0..8 {
        let (op, message) = reply(stream)?;
        if op == 3 {
            frame(stream, 4, &message)?;
        } else if op == 2 || message["evt"] == "ERROR" {
            return Err(io::Error::other("Discord rejected activity"));
        } else if message["nonce"] == nonce {
            return Ok(());
        }
    }
    Err(io::Error::other("No Discord RPC acknowledgement"))
}

#[cfg(windows)]
type Pipe = std::fs::File;

#[cfg(windows)]
fn connect() -> io::Result<Pipe> {
    (0..10)
        .find_map(|i| {
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(format!(r"\\.\pipe\discord-ipc-{i}"))
                .ok()
        })
        .ok_or_else(|| io::Error::other("Discord IPC unavailable"))
}

#[cfg(windows)]
fn read_exact(stream: &mut Pipe, bytes: &mut [u8]) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Pipes::PeekNamedPipe;
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let mut available = 0;
        if unsafe {
            PeekNamedPipe(
                stream.as_raw_handle(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut available,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if available as usize >= bytes.len() {
            return stream.read_exact(bytes);
        }
        if std::time::Instant::now() >= deadline {
            return Err(io::ErrorKind::TimedOut.into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(unix)]
type Pipe = std::os::unix::net::UnixStream;

#[cfg(unix)]
fn connect() -> io::Result<Pipe> {
    let folder = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"]
        .into_iter()
        .find_map(std::env::var_os)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/tmp".into());
    for i in 0..10 {
        for dir in [folder.clone(), folder.join("app/com.discordapp.Discord")] {
            if let Ok(pipe) = Pipe::connect(dir.join(format!("discord-ipc-{i}"))) {
                pipe.set_read_timeout(Some(Duration::from_secs(2)))?;
                pipe.set_write_timeout(Some(Duration::from_secs(2)))?;
                return Ok(pipe);
            }
        }
    }
    Err(io::Error::other("Discord IPC unavailable"))
}

#[cfg(unix)]
fn read_exact(stream: &mut Pipe, bytes: &mut [u8]) -> io::Result<()> {
    stream.read_exact(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "Requires the logged-in Discord desktop client"]
    fn desktop_acknowledges_launcher_activity() {
        let activity = Activity {
            application: crate::model::Config::default().discord_application,
            build: None,
        };
        let mut pipe = connect().unwrap();
        frame(
            &mut pipe,
            0,
            &json!({"v":1,"client_id":activity.application}),
        )
        .unwrap();
        assert_eq!(reply(&mut pipe).unwrap().1["evt"], "READY");
        publish(&mut pipe, &activity).unwrap();
    }
}
