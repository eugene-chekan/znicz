//! Localhost attach so MCP can use the TUI’s player.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{Result, ZniczError};
use crate::player::commands::Command;
use crate::player::engine::{PlayerHandle, PlayerOps};
use crate::player::state::PlayerState;

const CONNECT_TIMEOUT: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Advertise {
    port: u16,
    token: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum IpcRequest {
    State { token: String },
    Command { token: String, command: Command },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum IpcResponse {
    Ok { state: Box<PlayerState> },
    Err { message: String },
}

fn random_token() -> String {
    use std::hash::{BuildHasher, Hasher, RandomState};
    let mut a = RandomState::new().build_hasher();
    a.write_u64(std::process::id() as u64);
    a.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(1),
    );
    let mut b = RandomState::new().build_hasher();
    b.write_u64(a.finish());
    format!("{:016x}{:016x}", a.finish(), b.finish())
}

fn write_json_line(stream: &mut TcpStream, value: &impl Serialize) -> Result<()> {
    let mut line =
        serde_json::to_string(value).map_err(|e| ZniczError::Player(format!("ipc json: {e}")))?;
    line.push('\n');
    stream
        .write_all(line.as_bytes())
        .map_err(|e| ZniczError::Player(format!("ipc write: {e}")))?;
    stream
        .flush()
        .map_err(|e| ZniczError::Player(format!("ipc write: {e}")))?;
    Ok(())
}

fn read_json_line<T: for<'de> Deserialize<'de>>(stream: &TcpStream) -> Result<T> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|e| ZniczError::Player(format!("ipc read: {e}")))?;
    if line.is_empty() {
        return Err(ZniczError::Player("ipc empty".into()));
    }
    serde_json::from_str(line.trim()).map_err(|e| ZniczError::Player(format!("ipc json: {e}")))
}

fn load_advertise(path: &Path) -> Result<Advertise> {
    let text =
        fs::read_to_string(path).map_err(|e| ZniczError::Player(format!("ipc advertise: {e}")))?;
    toml::from_str(&text).map_err(|e| ZniczError::Player(format!("ipc advertise: {e}")))
}

fn save_advertise(path: &Path, advertise: &Advertise) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(advertise)
        .map_err(|e| ZniczError::Player(format!("ipc advertise: {e}")))?;
    fs::write(path, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// TUI-side listener. Removes the advertise file on drop.
pub struct IpcServer {
    advertise: PathBuf,
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl IpcServer {
    pub fn start(player: PlayerHandle, advertise: impl Into<PathBuf>) -> Result<Self> {
        let advertise = advertise.into();
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|e| ZniczError::Player(format!("ipc bind: {e}")))?;
        listener
            .set_nonblocking(false)
            .map_err(|e| ZniczError::Player(format!("ipc bind: {e}")))?;
        let addr = listener
            .local_addr()
            .map_err(|e| ZniczError::Player(format!("ipc bind: {e}")))?;
        let token = random_token();
        save_advertise(
            &advertise,
            &Advertise {
                port: addr.port(),
                token: token.clone(),
            },
        )?;

        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let join = match thread::Builder::new()
            .name("znicz-ipc".into())
            .spawn(move || serve_loop(listener, player, token, stop_thread))
        {
            Ok(join) => join,
            Err(e) => {
                let _ = fs::remove_file(&advertise);
                return Err(ZniczError::Player(format!("ipc thread: {e}")));
            }
        };

        Ok(Self {
            advertise,
            addr,
            stop,
            join: Some(join),
        })
    }
}

fn serve_loop(listener: TcpListener, player: PlayerHandle, token: String, stop: Arc<AtomicBool>) {
    listener.set_nonblocking(true).ok();
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                let _ = handle_conn(stream, &player, &token);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(_) => {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

fn handle_conn(stream: TcpStream, player: &PlayerHandle, token: &str) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(5))).ok();
    let req: IpcRequest = read_json_line(&stream)?;
    let (req_token, command) = match req {
        IpcRequest::State { token } => (token, None),
        IpcRequest::Command { token, command } => (token, Some(command)),
    };
    let mut stream = stream;
    if req_token != token {
        write_json_line(
            &mut stream,
            &IpcResponse::Err {
                message: "ipc token mismatch".into(),
            },
        )?;
        return Ok(());
    }
    let response = if let Some(command) = command {
        match player.send_blocking(command) {
            Ok(()) => IpcResponse::Ok {
                state: Box::new(player.state()),
            },
            Err(e) => IpcResponse::Err {
                message: e.to_string(),
            },
        }
    } else {
        IpcResponse::Ok {
            state: Box::new(player.state()),
        }
    };
    write_json_line(&mut stream, &response)
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_millis(200));
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
        let _ = fs::remove_file(&self.advertise);
    }
}

/// MCP-side client for one advertise file.
pub struct IpcClient {
    addr: SocketAddr,
    token: String,
}

impl IpcClient {
    pub fn connect(path: &Path) -> Result<Self> {
        let advertise = load_advertise(path)?;
        let addr = SocketAddr::from(([127, 0, 0, 1], advertise.port));
        let _probe = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
            .map_err(|e| ZniczError::Player(format!("ipc connect: {e}")))?;
        Ok(Self {
            addr,
            token: advertise.token,
        })
    }

    fn rpc(&self, request: IpcRequest) -> Result<IpcResponse> {
        let mut stream = TcpStream::connect_timeout(&self.addr, CONNECT_TIMEOUT)
            .map_err(|e| ZniczError::Player(format!("ipc connect: {e}")))?;
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
        stream.set_write_timeout(Some(Duration::from_secs(5))).ok();
        write_json_line(&mut stream, &request)?;
        read_json_line(&stream)
    }
}

impl PlayerOps for IpcClient {
    fn send_blocking(&self, command: Command) -> Result<()> {
        match self.rpc(IpcRequest::Command {
            token: self.token.clone(),
            command,
        })? {
            IpcResponse::Ok { .. } => Ok(()),
            IpcResponse::Err { message } => Err(ZniczError::Player(message)),
        }
    }

    fn state(&self) -> PlayerState {
        match self.rpc(IpcRequest::State {
            token: self.token.clone(),
        }) {
            Ok(IpcResponse::Ok { state }) => *state,
            Ok(IpcResponse::Err { message }) => {
                tracing::warn!("ipc state: {message}");
                PlayerState::default()
            }
            Err(e) => {
                tracing::warn!("ipc state: {e}");
                PlayerState::default()
            }
        }
    }
}

/// Snapshot from the TUI host, if it is up.
pub fn try_state(path: &Path) -> Result<PlayerState> {
    let client = IpcClient::connect(path)?;
    match client.rpc(IpcRequest::State {
        token: client.token.clone(),
    })? {
        IpcResponse::Ok { state } => Ok(*state),
        IpcResponse::Err { message } => Err(ZniczError::Player(message)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::engine::{spawn_player, AudioConfig};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn advertise_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "znicz-ipc-{}-{}.toml",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn client_sees_volume_set_on_the_host() {
        let path = advertise_path();
        let (player, _thread) = spawn_player(AudioConfig::default());
        let _server = IpcServer::start(player.clone(), &path).expect("start ipc");
        let client = IpcClient::connect(&path).expect("connect");
        client
            .send_blocking(Command::SetVolume(0.4))
            .expect("set volume");
        assert!((client.state().volume - 0.4).abs() < 0.001);
        assert!((player.state().volume - 0.4).abs() < 0.001);
    }

    #[test]
    fn missing_advertise_is_an_error() {
        let path = advertise_path();
        let _ = fs::remove_file(&path);
        assert!(IpcClient::connect(&path).is_err());
        assert!(try_state(&path).is_err());
    }

    #[test]
    fn wrong_token_does_not_change_the_host() {
        let path = advertise_path();
        let (player, _thread) = spawn_player(AudioConfig::default());
        let _server = IpcServer::start(player.clone(), &path).expect("start ipc");
        let mut client = IpcClient::connect(&path).expect("connect");
        client.token = "nope".into();
        let err = client
            .send_blocking(Command::SetVolume(0.2))
            .expect_err("token");
        assert!(err.to_string().contains("token"), "{err}");
        assert!((player.state().volume - 1.0).abs() < 0.001);
    }
}
