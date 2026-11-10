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
use crate::library_host::{LibraryHost, LibraryOp, LibraryStatus};
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
    Library { token: String, op: LibraryOp },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum IpcResponse {
    Ok {
        state: Box<PlayerState>,
        /// The library writer's status, when this player hosts one.
        #[serde(default)]
        library: Option<LibraryStatus>,
    },
    Err {
        message: String,
    },
}

struct Shared {
    player: PlayerHandle,
    token: String,
    ui_count: AtomicUsize,
    stop: Arc<AtomicBool>,
    idle: Duration,
    advertise: PathBuf,
    persist: Mutex<Option<SessionPersister>>,
    library: Option<Arc<dyn LibraryHost>>,
}

impl Shared {
    fn ok(&self) -> IpcResponse {
        IpcResponse::Ok {
            state: Box::new(self.player.state()),
            library: self.library.as_ref().map(|host| host.status()),
        }
    }
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
        Self::start_inner(player, advertise, idle, None, None)
    }

    pub fn start_with_session(
        player: PlayerHandle,
        advertise: impl Into<PathBuf>,
        idle: Duration,
        session_path: impl Into<PathBuf>,
        debounce: Duration,
    ) -> Result<Self> {
        Self::start_with_session_and_library(player, advertise, idle, session_path, debounce, None)
    }

    /// A server whose clients can also write the library through `host`.
    pub fn start_with_library(
        player: PlayerHandle,
        advertise: impl Into<PathBuf>,
        idle: Duration,
        host: Arc<dyn LibraryHost>,
    ) -> Result<Self> {
        Self::start_inner(player, advertise, idle, None, Some(host))
    }

    /// The `znicz player` setup: session persistence plus an optional library writer.
    pub fn start_with_session_and_library(
        player: PlayerHandle,
        advertise: impl Into<PathBuf>,
        idle: Duration,
        session_path: impl Into<PathBuf>,
        debounce: Duration,
        host: Option<Arc<dyn LibraryHost>>,
    ) -> Result<Self> {
        let mut persister = SessionPersister::new(session_path, debounce);
        persister.sync_from_player(&player);
        Self::start_inner(player, advertise, idle, Some(persister), host)
    }

    fn start_inner(
        player: PlayerHandle,
        advertise: impl Into<PathBuf>,
        idle: Duration,
        persist: Option<SessionPersister>,
        library: Option<Arc<dyn LibraryHost>>,
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
            library,
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

    pub fn stop_flag(&self) -> Arc<AtomicBool> {
        self.stop.clone()
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
    // Only ask the library when nothing else keeps the player up.
    let scanning = || {
        shared
            .library
            .as_ref()
            .is_some_and(|host| host.status().busy())
    };
    if stopped && no_ui && !scanning() {
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
    let _ = write_json_line(&mut writer, &shared.ok());
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
            | IpcRequest::Shutdown { token }
            | IpcRequest::Library { token, .. } => token.clone(),
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
                let _ = write_json_line(&mut writer, &shared.ok());
            }
            IpcRequest::Command { command, .. } => {
                let response = match shared.player.send_blocking(command) {
                    Ok(()) => shared.ok(),
                    Err(e) => IpcResponse::Err {
                        message: e.to_string(),
                    },
                };
                let _ = write_json_line(&mut writer, &response);
            }
            IpcRequest::Library { op, .. } => {
                let response = match &shared.library {
                    None => IpcResponse::Err {
                        message: "no music library configured".into(),
                    },
                    Some(host) => match host.apply(op) {
                        Ok(()) => shared.ok(),
                        Err(e) => IpcResponse::Err {
                            message: e.to_string(),
                        },
                    },
                };
                let _ = write_json_line(&mut writer, &response);
            }
            IpcRequest::Shutdown { .. } => {
                persist_flush(&shared);
                let _ = write_json_line(&mut writer, &shared.ok());
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

/// An old player drops the connection on a request it does not know; say so.
fn outdated_player_hint(e: impl std::fmt::Display) -> ZniczError {
    ZniczError::Library(format!(
        "{e} (the running player may be an older version; run `znicz player stop` and try again)"
    ))
}

/// A failed rpc, and whether a connection was open when it failed.
struct RpcFailure {
    error: ZniczError,
    /// The request was sent (or tried) on an open connection. Errors from
    /// (re)opening the connection, such as "connection refused", are not.
    mid_request: bool,
}

impl RpcFailure {
    fn connect(error: ZniczError) -> Self {
        Self {
            error,
            mid_request: false,
        }
    }

    /// The outdated-player hint only fits a request that died on an open
    /// connection; a player that is simply not running stays a plain error.
    fn into_error(self) -> ZniczError {
        if self.mid_request {
            outdated_player_hint(self.error)
        } else {
            self.error
        }
    }
}

/// Map a command rpc outcome: a request that dies on an open connection gets
/// the outdated-player hint, the player's own errors pass through verbatim.
fn command_result(response: std::result::Result<IpcResponse, RpcFailure>) -> Result<()> {
    match response.map_err(RpcFailure::into_error)? {
        IpcResponse::Ok { .. } => Ok(()),
        IpcResponse::Err { message } => Err(ZniczError::Player(message)),
    }
}

impl LibraryHost for IpcClient {
    fn apply(&self, op: LibraryOp) -> Result<()> {
        match self
            .rpc_detailed(|token| IpcRequest::Library {
                token: token.to_string(),
                op: op.clone(),
            })
            .map_err(RpcFailure::into_error)?
        {
            IpcResponse::Ok { .. } => Ok(()),
            IpcResponse::Err { message } => Err(ZniczError::Library(message)),
        }
    }

    /// Fresh from the player; default when it hosts no library or is unreachable.
    fn status(&self) -> LibraryStatus {
        self.try_status().unwrap_or_default()
    }

    fn try_status(&self) -> Result<LibraryStatus> {
        match self.rpc(|token| IpcRequest::State {
            token: token.to_string(),
        })? {
            IpcResponse::Ok { library, .. } => Ok(library.unwrap_or_default()),
            IpcResponse::Err { message } => Err(ZniczError::Library(message)),
        }
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

/// Client for one advertise file. Re-reads that file if the host restarts.
#[derive(Clone)]
pub struct IpcClient {
    inner: Arc<IpcInner>,
}

struct Live {
    writer: TcpStream,
    reader: BufReader<TcpStream>,
    token: String,
}

type EnsureFn = Arc<dyn Fn() -> Result<()> + Send + Sync>;

struct IpcInner {
    advertise: PathBuf,
    role: ClientRole,
    live: Mutex<Live>,
    ensure: Option<EnsureFn>,
}

fn open_stream(advertise_path: &Path, role: ClientRole) -> Result<Live> {
    let advertise = load_advertise(advertise_path)?;
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
    Ok(Live {
        writer,
        reader,
        token: advertise.token,
    })
}

fn open_live(advertise_path: &Path, role: ClientRole, ensure: Option<&EnsureFn>) -> Result<Live> {
    match open_stream(advertise_path, role) {
        Ok(live) => Ok(live),
        Err(first) => {
            let Some(ensure) = ensure else {
                return Err(first);
            };
            ensure()?;
            open_stream(advertise_path, role)
        }
    }
}

fn rpc_on(live: &mut Live, request: &IpcRequest) -> Result<IpcResponse> {
    write_json_line(&mut live.writer, request)?;
    read_json_line(&mut live.reader)
}

impl IpcClient {
    pub fn connect(path: impl AsRef<Path>, role: ClientRole) -> Result<Self> {
        Self::connect_inner(path.as_ref().to_path_buf(), role, None)
    }

    pub fn connect_with_ensure(
        path: impl AsRef<Path>,
        role: ClientRole,
        ensure: impl Fn() -> Result<()> + Send + Sync + 'static,
    ) -> Result<Self> {
        Self::connect_inner(path.as_ref().to_path_buf(), role, Some(Arc::new(ensure)))
    }

    fn connect_inner(
        advertise: PathBuf,
        role: ClientRole,
        ensure: Option<EnsureFn>,
    ) -> Result<Self> {
        let live = open_live(&advertise, role, ensure.as_ref())?;
        Ok(Self {
            inner: Arc::new(IpcInner {
                advertise,
                role,
                live: Mutex::new(live),
                ensure,
            }),
        })
    }

    fn lock_live(&self) -> Result<std::sync::MutexGuard<'_, Live>> {
        self.inner
            .live
            .lock()
            .map_err(|_| ZniczError::Player("ipc lock".into()))
    }

    fn rpc(&self, make: impl Fn(&str) -> IpcRequest) -> Result<IpcResponse> {
        self.rpc_detailed(make).map_err(|f| f.error)
    }

    fn rpc_detailed(
        &self,
        make: impl Fn(&str) -> IpcRequest,
    ) -> std::result::Result<IpcResponse, RpcFailure> {
        let mut live = self.lock_live().map_err(RpcFailure::connect)?;
        let request = make(&live.token);
        match rpc_on(&mut live, &request) {
            Ok(response) => Ok(response),
            Err(_) => {
                *live = open_live(
                    &self.inner.advertise,
                    self.inner.role,
                    self.inner.ensure.as_ref(),
                )
                .map_err(RpcFailure::connect)?;
                let request = make(&live.token);
                rpc_on(&mut live, &request).map_err(|error| RpcFailure {
                    error,
                    mid_request: true,
                })
            }
        }
    }

    #[cfg(test)]
    fn rpc_raw(&self, request: IpcRequest) -> Result<IpcResponse> {
        let mut live = self.lock_live()?;
        rpc_on(&mut live, &request)
    }

    pub fn shutdown(&self) -> Result<()> {
        let mut live = self.lock_live()?;
        let token = live.token.clone();
        match rpc_on(&mut live, &IpcRequest::Shutdown { token })? {
            IpcResponse::Ok { .. } => Ok(()),
            IpcResponse::Err { message } => Err(ZniczError::Player(message)),
        }
    }

    pub fn send_blocking(&self, command: Command) -> Result<()> {
        command_result(self.rpc_detailed(|token| IpcRequest::Command {
            token: token.to_string(),
            command: command.clone(),
        }))
    }

    pub fn state(&self) -> Result<PlayerState> {
        match self.rpc(|token| IpcRequest::State {
            token: token.to_string(),
        })? {
            IpcResponse::Ok { state, .. } => Ok(*state),
            IpcResponse::Err { message } => Err(ZniczError::Player(message)),
        }
    }
}

impl PlayerOps for IpcClient {
    fn send_blocking(&self, command: Command) -> Result<()> {
        IpcClient::send_blocking(self, command)
    }

    fn state(&self) -> PlayerState {
        IpcClient::state(self).unwrap_or_else(|e| {
            tracing::warn!("ipc state: {e}");
            PlayerState::default()
        })
    }
}

/// Snapshot from the player host, if it is up.
pub fn try_state(path: &Path) -> Result<PlayerState> {
    IpcClient::connect(path, ClientRole::Agent)?.state()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::engine::{spawn_player, AudioConfig};
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn command_transport_errors_get_the_hint_and_player_errors_stay_verbatim() {
        let transport = command_result(Err(RpcFailure {
            error: ZniczError::Player("broken pipe".into()),
            mid_request: true,
        }))
        .unwrap_err()
        .to_string();
        assert!(transport.contains("broken pipe"), "{transport}");
        assert!(transport.contains("znicz player stop"), "{transport}");

        let connect = command_result(Err(RpcFailure {
            error: ZniczError::Player("ipc connect: connection refused".into()),
            mid_request: false,
        }))
        .unwrap_err()
        .to_string();
        assert!(connect.contains("connection refused"), "{connect}");
        assert!(!connect.contains("znicz player stop"), "{connect}");

        let player = command_result(Ok(IpcResponse::Err {
            message: "queue index out of range".into(),
        }))
        .unwrap_err()
        .to_string();
        assert!(player.contains("queue index out of range"), "{player}");
        assert!(!player.contains("znicz player stop"), "{player}");
    }

    #[test]
    fn a_transport_failure_suggests_restarting_the_player() {
        let msg = outdated_player_hint("connection reset").to_string();
        assert!(msg.contains("connection reset"), "{msg}");
        assert!(msg.contains("older version"), "{msg}");
        assert!(msg.contains("znicz player stop"), "{msg}");
    }

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
        assert!((client.state().expect("state").volume - 0.4).abs() < 0.001);
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
                .rpc_raw(IpcRequest::Command {
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

    #[test]
    fn state_after_host_exit_is_an_error_not_defaults() {
        let path = advertise_path();
        let (player, _thread) = spawn_player(AudioConfig::default());
        let _server = IpcServer::start(player, &path).expect("start");
        let client = IpcClient::connect(&path, ClientRole::Agent).expect("connect");
        client.shutdown().expect("shutdown");
        thread::sleep(Duration::from_millis(50));
        let err = client
            .state()
            .expect_err("dead socket must not look like a Stopped player");
        let message = err.to_string();
        assert!(
            message.contains("ipc") || message.contains("player"),
            "{message}"
        );
    }

    #[test]
    fn client_follows_a_new_host_on_the_same_advertise() {
        let path = advertise_path();
        let (player_a, _thread_a) = spawn_player(AudioConfig::default());
        player_a
            .send_blocking(Command::SetVolume(0.2))
            .expect("vol a");
        let _server_a = IpcServer::start(player_a, &path).expect("start a");
        let client = IpcClient::connect(&path, ClientRole::Agent).expect("connect");
        client.shutdown().expect("stop a");
        thread::sleep(Duration::from_millis(50));

        let (player_b, _thread_b) = spawn_player(AudioConfig::default());
        player_b
            .send_blocking(Command::SetVolume(0.5))
            .expect("vol b");
        let _server_b = IpcServer::start(player_b, &path).expect("start b");
        let state = client.state().expect("reconnect to the new host");
        assert!((state.volume - 0.5).abs() < 0.001);
    }

    #[test]
    fn ensure_callback_starts_a_host_after_idle_exit() {
        let path = advertise_path();
        let (player_a, _thread_a) = spawn_player(AudioConfig::default());
        player_a
            .send_blocking(Command::SetVolume(0.2))
            .expect("vol a");
        let _server_a = IpcServer::start(player_a, &path).expect("start a");

        let holder: std::sync::Arc<Mutex<Option<IpcServer>>> =
            std::sync::Arc::new(Mutex::new(None));
        let (player_b, _thread_b) = spawn_player(AudioConfig::default());
        player_b
            .send_blocking(Command::SetVolume(0.7))
            .expect("vol b");
        let holder2 = holder.clone();
        let player_b2 = player_b.clone();
        let path2 = path.clone();
        let client = IpcClient::connect_with_ensure(&path, ClientRole::Agent, move || {
            let server = IpcServer::start(player_b2.clone(), &path2)
                .map_err(|e| ZniczError::Player(e.to_string()))?;
            *holder2
                .lock()
                .map_err(|_| ZniczError::Player("lock".into()))? = Some(server);
            Ok(())
        })
        .expect("connect");
        assert!((client.state().expect("state").volume - 0.2).abs() < 0.001);

        client.shutdown().expect("stop a");
        thread::sleep(Duration::from_millis(50));
        let state = client.state().expect("ensure started host b");
        assert!((state.volume - 0.7).abs() < 0.001);
    }

    use crate::library_host::{LibraryHost, LibraryOp, LibraryStatus, ScanProgress};

    #[derive(Default)]
    struct FakeHost {
        ops: Mutex<Vec<LibraryOp>>,
        busy: AtomicBool,
    }

    impl LibraryHost for FakeHost {
        fn apply(&self, op: LibraryOp) -> Result<()> {
            if op == LibraryOp::Prune {
                return Err(ZniczError::Library("refused".into()));
            }
            self.ops.lock().unwrap().push(op);
            Ok(())
        }
        fn status(&self) -> LibraryStatus {
            LibraryStatus {
                scans_finished: self.ops.lock().unwrap().len() as u64,
                scanning: self.busy.load(Ordering::SeqCst).then(ScanProgress::default),
                ..LibraryStatus::default()
            }
        }
    }

    #[test]
    fn library_ops_reach_the_host_and_status_comes_back() {
        let path = advertise_path();
        let (player, _thread) = spawn_player(AudioConfig::default());
        let host = Arc::new(FakeHost::default());
        let _server = IpcServer::start_with_library(player, &path, Duration::ZERO, host.clone())
            .expect("start ipc");
        let client = IpcClient::connect(&path, ClientRole::Agent).expect("connect");

        client
            .apply(LibraryOp::AddFolder {
                path: PathBuf::from("/music"),
            })
            .expect("apply");
        assert_eq!(host.ops.lock().unwrap().len(), 1);
        assert_eq!(client.status().scans_finished, 1);

        let err = client.apply(LibraryOp::Prune).expect_err("host refused");
        assert_eq!(err.to_string(), "refused");
    }

    #[test]
    fn without_a_host_library_requests_fail() {
        let path = advertise_path();
        let (player, _thread) = spawn_player(AudioConfig::default());
        let _server = IpcServer::start(player, &path).expect("start ipc");
        let client = IpcClient::connect(&path, ClientRole::Agent).expect("connect");
        let err = client.apply(LibraryOp::RescanAll).expect_err("no host");
        assert!(err.to_string().contains("no music library"), "{err}");
        assert_eq!(client.status(), LibraryStatus::default());
    }

    #[test]
    fn a_running_scan_holds_off_idle_exit() {
        let path = advertise_path();
        let (player, _thread) = spawn_player(AudioConfig::default());
        let host = Arc::new(FakeHost::default());
        host.busy.store(true, Ordering::SeqCst);
        let _server =
            IpcServer::start_with_library(player, &path, Duration::from_millis(80), host.clone())
                .expect("start ipc");
        thread::sleep(Duration::from_millis(500));
        assert!(path.exists(), "must not idle-exit while scanning");
        host.busy.store(false, Ordering::SeqCst);
        let start = Instant::now();
        while path.exists() && start.elapsed() < Duration::from_secs(3) {
            thread::sleep(Duration::from_millis(50));
        }
        assert!(!path.exists(), "idles out once the scan is done");
    }
}
