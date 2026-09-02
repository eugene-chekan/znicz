//! Localhost player socket. The player process hosts; TUI and MCP are clients.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::error::{Result, ZniczError};
use crate::player::commands::Command;
use crate::player::engine::{PlayerHandle, PlayerOps};
use crate::player::state::{PlaybackStatus, PlayerState};
use crate::session::SessionPersister;

const CONNECT_TIMEOUT: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientRole {
    Ui,
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Advertise {
    port: u16,
    token: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum IpcRequest {
    Hello { token: String, role: ClientRole },
    State { token: String },
    Command { token: String, command: Command },
    Shutdown { token: String },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum IpcResponse {
    Ok { state: Box<PlayerState> },
    Err { message: String },
}

struct Shared {
    player: PlayerHandle,
    token: String,
    ui_count: AtomicUsize,
    stop: Arc<AtomicBool>,
    idle: Duration,
    advertise: PathBuf,
    persist: Mutex<Option<SessionPersister>>,
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

fn write_json_line(stream: &mut impl Write, value: &impl Serialize) -> Result<()> {
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

fn read_json_line<T: for<'de> Deserialize<'de>>(
    reader: &mut BufReader<impl std::io::Read>,
) -> Result<T> {
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

/// Player-side listener. Removes the advertise file on stop or drop.
pub struct IpcServer {
    advertise: PathBuf,
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl IpcServer {
    pub fn start(player: PlayerHandle, advertise: impl Into<PathBuf>) -> Result<Self> {
        Self::start_with_idle(player, advertise, Duration::ZERO)
    }

    pub fn start_with_idle(
        player: PlayerHandle,
        advertise: impl Into<PathBuf>,
        idle: Duration,
    ) -> Result<Self> {
        Self::start_inner(player, advertise, idle, None)
    }

    pub fn start_with_session(
        player: PlayerHandle,
        advertise: impl Into<PathBuf>,
        idle: Duration,
        session_path: impl Into<PathBuf>,
        debounce: Duration,
    ) -> Result<Self> {
        let mut persister = SessionPersister::new(session_path, debounce);
        persister.sync_from_player(&player);
        Self::start_inner(player, advertise, idle, Some(persister))
    }

    fn start_inner(
        player: PlayerHandle,
        advertise: impl Into<PathBuf>,
        idle: Duration,
        persist: Option<SessionPersister>,
    ) -> Result<Self> {
        let advertise = advertise.into();
        let listener = TcpListener::bind("127.0.0.1:0")
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
        let shared = Arc::new(Shared {
            player,
            token,
            ui_count: AtomicUsize::new(0),
            stop: stop.clone(),
            idle,
            advertise: advertise.clone(),
            persist: Mutex::new(persist),
        });
        let join = match thread::Builder::new()
            .name("znicz-ipc".into())
            .spawn(move || serve_loop(listener, shared))
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

    pub fn wait(&mut self) {
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn serve_loop(listener: TcpListener, shared: Arc<Shared>) {
    listener.set_nonblocking(true).ok();
    let mut idle_since: Option<Instant> = None;
    while !shared.stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                let session = shared.clone();
                let _ = thread::Builder::new()
                    .name("znicz-ipc-conn".into())
                    .spawn(move || handle_session(stream, session));
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => {
                thread::sleep(Duration::from_millis(20));
                continue;
            }
        }

        persist_tick(&shared);

        if should_idle_exit(&shared, &mut idle_since) {
            persist_flush(&shared);
            shared.stop.store(true, Ordering::SeqCst);
            let _ = fs::remove_file(&shared.advertise);
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    persist_flush(&shared);
}

fn persist_tick(shared: &Shared) {
    let Ok(mut guard) = shared.persist.lock() else {
        return;
    };
    let Some(persister) = guard.as_mut() else {
        return;
    };
    if let Err(e) = persister.tick(&shared.player) {
        tracing::warn!(error = %e, "session.toml");
    }
}

fn persist_flush(shared: &Shared) {
    let Ok(mut guard) = shared.persist.lock() else {
        return;
    };
    let Some(persister) = guard.as_mut() else {
        return;
    };
    if let Err(e) = persister.flush(&shared.player) {
        tracing::warn!(error = %e, "session.toml");
    }
}

fn should_idle_exit(shared: &Shared, idle_since: &mut Option<Instant>) -> bool {
    if shared.idle.is_zero() {
        *idle_since = None;
        return false;
    }
    let stopped = shared.player.state().status == PlaybackStatus::Stopped;
    let no_ui = shared.ui_count.load(Ordering::SeqCst) == 0;
    if stopped && no_ui {
        let since = idle_since.get_or_insert_with(Instant::now);
        since.elapsed() >= shared.idle
    } else {
        *idle_since = None;
        false
    }
}

fn handle_session(stream: TcpStream, shared: Arc<Shared>) {
    let _ = stream.set_nonblocking(false);
    let mut writer = match stream.try_clone() {
        Ok(w) => w,
        Err(_) => return,
    };
    let mut reader = BufReader::new(stream);
    let hello: IpcRequest = match read_json_line(&mut reader) {
        Ok(h) => h,
        Err(_) => return,
    };
    let IpcRequest::Hello { token, role } = hello else {
        let _ = write_json_line(
            &mut writer,
            &IpcResponse::Err {
                message: "ipc hello required".into(),
            },
        );
        return;
    };
    if token != shared.token {
        let _ = write_json_line(
            &mut writer,
            &IpcResponse::Err {
                message: "ipc token mismatch".into(),
            },
        );
        return;
    }
    let _ = write_json_line(
        &mut writer,
        &IpcResponse::Ok {
            state: Box::new(shared.player.state()),
        },
    );
    let is_ui = role == ClientRole::Ui;
    if is_ui {
        shared.ui_count.fetch_add(1, Ordering::SeqCst);
    }
    loop {
        if shared.stop.load(Ordering::SeqCst) {
            break;
        }
        let req: IpcRequest = match read_json_line(&mut reader) {
            Ok(r) => r,
            Err(_) => break,
        };
        let req_token = match &req {
            IpcRequest::Hello { token, .. }
            | IpcRequest::State { token }
            | IpcRequest::Command { token, .. }
            | IpcRequest::Shutdown { token } => token.clone(),
        };
        if req_token != shared.token {
            let _ = write_json_line(
                &mut writer,
                &IpcResponse::Err {
                    message: "ipc token mismatch".into(),
                },
            );
            continue;
        }
        match req {
            IpcRequest::Hello { .. } => {
                let _ = write_json_line(
                    &mut writer,
                    &IpcResponse::Err {
                        message: "ipc already hello".into(),
                    },
                );
            }
            IpcRequest::State { .. } => {
                let _ = write_json_line(
                    &mut writer,
                    &IpcResponse::Ok {
                        state: Box::new(shared.player.state()),
                    },
                );
            }
            IpcRequest::Command { command, .. } => {
                let response = match shared.player.send_blocking(command) {
                    Ok(()) => IpcResponse::Ok {
                        state: Box::new(shared.player.state()),
                    },
                    Err(e) => IpcResponse::Err {
                        message: e.to_string(),
                    },
                };
                let _ = write_json_line(&mut writer, &response);
            }
            IpcRequest::Shutdown { .. } => {
                persist_flush(&shared);
                let _ = write_json_line(
                    &mut writer,
                    &IpcResponse::Ok {
                        state: Box::new(shared.player.state()),
                    },
                );
                shared.stop.store(true, Ordering::SeqCst);
                let _ = fs::remove_file(&shared.advertise);
                break;
            }
        }
    }
    if is_ui {
        shared.ui_count.fetch_sub(1, Ordering::SeqCst);
    }
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

/// Client for one advertise file. Holds the TCP session after Hello.
#[derive(Clone)]
pub struct IpcClient {
    inner: Arc<IpcInner>,
}

struct IpcInner {
    writer: Mutex<TcpStream>,
    reader: Mutex<BufReader<TcpStream>>,
    token: String,
}

impl IpcClient {
    pub fn connect(path: &Path, role: ClientRole) -> Result<Self> {
        let advertise = load_advertise(path)?;
        let addr = SocketAddr::from(([127, 0, 0, 1], advertise.port));
        let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
            .map_err(|e| ZniczError::Player(format!("ipc connect: {e}")))?;
        stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
        stream.set_write_timeout(Some(Duration::from_secs(5))).ok();
        let reader_stream = stream
            .try_clone()
            .map_err(|e| ZniczError::Player(format!("ipc connect: {e}")))?;
        let mut writer = stream;
        let mut reader = BufReader::new(reader_stream);
        write_json_line(
            &mut writer,
            &IpcRequest::Hello {
                token: advertise.token.clone(),
                role,
            },
        )?;
        match read_json_line::<IpcResponse>(&mut reader)? {
            IpcResponse::Ok { .. } => {}
            IpcResponse::Err { message } => return Err(ZniczError::Player(message)),
        }
        Ok(Self {
            inner: Arc::new(IpcInner {
                writer: Mutex::new(writer),
                reader: Mutex::new(reader),
                token: advertise.token,
            }),
        })
    }

    fn rpc(&self, request: IpcRequest) -> Result<IpcResponse> {
        let mut writer = self
            .inner
            .writer
            .lock()
            .map_err(|_| ZniczError::Player("ipc lock".into()))?;
        let mut reader = self
            .inner
            .reader
            .lock()
            .map_err(|_| ZniczError::Player("ipc lock".into()))?;
        write_json_line(&mut *writer, &request)?;
        read_json_line(&mut *reader)
    }

    pub fn shutdown(&self) -> Result<()> {
        match self.rpc(IpcRequest::Shutdown {
            token: self.inner.token.clone(),
        })? {
            IpcResponse::Ok { .. } => Ok(()),
            IpcResponse::Err { message } => Err(ZniczError::Player(message)),
        }
    }

    pub fn send_blocking(&self, command: Command) -> Result<()> {
        match self.rpc(IpcRequest::Command {
            token: self.inner.token.clone(),
            command,
        })? {
            IpcResponse::Ok { .. } => Ok(()),
            IpcResponse::Err { message } => Err(ZniczError::Player(message)),
        }
    }

    pub fn state(&self) -> PlayerState {
        match self.rpc(IpcRequest::State {
            token: self.inner.token.clone(),
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

impl PlayerOps for IpcClient {
    fn send_blocking(&self, command: Command) -> Result<()> {
        IpcClient::send_blocking(self, command)
    }

    fn state(&self) -> PlayerState {
        IpcClient::state(self)
    }
}

/// Snapshot from the player host, if it is up.
pub fn try_state(path: &Path) -> Result<PlayerState> {
    let client = IpcClient::connect(path, ClientRole::Agent)?;
    Ok(client.state())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::engine::{spawn_player, AudioConfig};
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn advertise_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "znicz-ipc-{}-{}.toml",
            std::process::id(),
            NEXT.fetch_add(1, AtomicOrdering::Relaxed)
        ))
    }

    #[test]
    fn client_sees_volume_set_on_the_host() {
        let path = advertise_path();
        let (player, _thread) = spawn_player(AudioConfig::default());
        let _server = IpcServer::start(player.clone(), &path).expect("start ipc");
        let client = IpcClient::connect(&path, ClientRole::Agent).expect("connect");
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
        assert!(IpcClient::connect(&path, ClientRole::Agent).is_err());
        assert!(try_state(&path).is_err());
    }

    #[test]
    fn wrong_token_does_not_change_the_host() {
        let path = advertise_path();
        let (player, _thread) = spawn_player(AudioConfig::default());
        let _server = IpcServer::start(player.clone(), &path).expect("start ipc");
        let client = IpcClient::connect(&path, ClientRole::Agent).expect("connect");
        {
            let token = String::from("nope");
            let err = client
                .rpc(IpcRequest::Command {
                    token,
                    command: Command::SetVolume(0.2),
                })
                .expect("rpc");
            match err {
                IpcResponse::Err { message } => assert!(message.contains("token"), "{message}"),
                other => panic!("expected token error, got {other:?}"),
            }
        }
        assert!((player.state().volume - 1.0).abs() < 0.001);
    }

    #[test]
    fn ui_hello_blocks_stopped_idle() {
        let path = advertise_path();
        let (player, _thread) = spawn_player(AudioConfig::default());
        let _server =
            IpcServer::start_with_idle(player, &path, Duration::from_millis(80)).expect("start");
        let _ui = IpcClient::connect(&path, ClientRole::Ui).expect("ui");
        thread::sleep(Duration::from_millis(200));
        IpcClient::connect(&path, ClientRole::Agent).expect("still up");
    }

    #[test]
    fn agent_only_does_not_block_stopped_idle() {
        let path = advertise_path();
        let (player, _thread) = spawn_player(AudioConfig::default());
        let _server =
            IpcServer::start_with_idle(player, &path, Duration::from_millis(80)).expect("start");
        let _agent = IpcClient::connect(&path, ClientRole::Agent).expect("agent");
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(2) {
            if IpcClient::connect(&path, ClientRole::Agent).is_err() {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("player should idle-exit with only an agent connected");
    }

    #[test]
    fn playing_does_not_idle_without_ui() {
        let path = advertise_path();
        let (player, _thread) = spawn_player(AudioConfig::default());
        {
            let arc = player.state_arc();
            let mut state = arc.write().unwrap();
            state.status = PlaybackStatus::Playing;
        }
        let _server =
            IpcServer::start_with_idle(player, &path, Duration::from_millis(80)).expect("start");
        thread::sleep(Duration::from_millis(200));
        IpcClient::connect(&path, ClientRole::Agent).expect("still up while playing");
    }

    #[test]
    fn shutdown_stops_the_server() {
        let path = advertise_path();
        let (player, _thread) = spawn_player(AudioConfig::default());
        let _server = IpcServer::start(player, &path).expect("start");
        let client = IpcClient::connect(&path, ClientRole::Agent).expect("connect");
        client.shutdown().expect("shutdown");
        thread::sleep(Duration::from_millis(150));
        assert!(IpcClient::connect(&path, ClientRole::Agent).is_err());
    }

    fn session_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "znicz-ipc-session-{}-{}.toml",
            std::process::id(),
            NEXT.fetch_add(1, AtomicOrdering::Relaxed)
        ))
    }

    #[test]
    fn host_writes_session_after_client_mutes() {
        let path = advertise_path();
        let session = session_path();
        let (player, _thread) = spawn_player(AudioConfig::default());
        let _server = IpcServer::start_with_session(
            player,
            &path,
            Duration::ZERO,
            &session,
            Duration::from_millis(30),
        )
        .expect("start");
        let client = IpcClient::connect(&path, ClientRole::Agent).expect("connect");
        client.send_blocking(Command::SetMuted(true)).expect("mute");
        thread::sleep(Duration::from_millis(20));
        assert!(
            !session.is_file() || !crate::session::load(&session).expect("load").muted,
            "must not write before debounce"
        );
        let start = Instant::now();
        loop {
            if session.is_file() && crate::session::load(&session).expect("load").muted {
                break;
            }
            if start.elapsed() > Duration::from_secs(2) {
                panic!("session.toml should record mute after debounce");
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}
