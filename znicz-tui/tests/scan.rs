//! Scanning from the TUI through an in-process library service.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use znicz_core::{spawn_player, wait_for_library, AudioConfig, LibraryHost};
use znicz_library::{Library, LibraryService};
use znicz_tui::{views, App};

struct Setup {
    app: App,
    root: PathBuf,
    service: Arc<LibraryService>,
    // Keeps the temp folder alive, and removes it, for the whole test.
    _dir: tempfile::TempDir,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let db = root.join("library.db");
    let service = Arc::new(LibraryService::open(&db).unwrap());
    let (player, _thread) = spawn_player(AudioConfig::default());
    let mut app = App::with_library(player, Some(Library::open(&db).unwrap()));
    app.library_host = Some(service.clone());
    app.library_poll_every = Duration::ZERO;
    Setup {
        app,
        root,
        service,
        _dir: dir,
    }
}

fn press(app: &mut App, code: KeyCode) {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        press(app, KeyCode::Char(c));
    }
}

fn draw(app: &mut App) -> String {
    let state = app.state();
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|frame| views::render(frame, app, &state))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let mut out = String::new();
    for y in 0..buffer.area().height {
        for x in 0..buffer.area().width {
            out.push_str(buffer[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

fn wait(service: &LibraryService, target: u64) {
    wait_for_library(service, target, Some(Duration::from_secs(10)), |_| {}).expect("scan");
}

fn music(root: &Path) -> PathBuf {
    let dir = root.join("music");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn an_empty_library_opens_the_folder_prompt() {
    let mut s = setup();
    let screen = draw(&mut s.app);
    assert!(s.app.folder_prompt.is_some());
    assert!(
        screen.contains("No music yet. Which folder holds your music?"),
        "{screen}"
    );
    assert!(screen.contains("folder: "), "{screen}");
}

#[test]
fn the_folder_prompt_does_not_open_over_another_prompt() {
    use znicz_tui::line_edit::LineEdit;
    use znicz_tui::PlaylistPrompt;
    let mut s = setup();
    s.app.playlist_prompt = Some(PlaylistPrompt::Save(LineEdit::new()));
    draw(&mut s.app);
    assert!(s.app.folder_prompt.is_none());
    s.app.playlist_prompt = None;
    s.app.radio_prompt = Some(znicz_tui::RadioPrompt::Copy(LineEdit::new()));
    draw(&mut s.app);
    assert!(s.app.folder_prompt.is_none());
}

#[test]
fn esc_skips_the_prompt_for_the_session() {
    let mut s = setup();
    draw(&mut s.app);
    press(&mut s.app, KeyCode::Esc);
    let screen = draw(&mut s.app);
    assert!(s.app.folder_prompt.is_none(), "does not reopen");
    assert!(screen.contains("press F to add a folder"), "{screen}");
}

#[test]
fn enter_adds_the_folder_and_the_library_reloads_when_done() {
    let mut s = setup();
    let music = music(&s.root);
    draw(&mut s.app);
    s.app.folder_prompt = Some(znicz_tui::line_edit::LineEdit::new());
    type_text(&mut s.app, &music.display().to_string());
    press(&mut s.app, KeyCode::Enter);
    assert!(s.app.folder_prompt.is_none());
    wait(&s.service, 1);
    draw(&mut s.app);
    assert_eq!(s.app.library_status.folders, [music]);
    let message = s.app.toasts.current().expect("finish message");
    assert!(message.text.starts_with("scan done: "), "{}", message.text);
}

#[test]
fn a_path_that_is_not_a_folder_keeps_the_prompt_open() {
    let mut s = setup();
    draw(&mut s.app);
    s.app.folder_prompt = Some(znicz_tui::line_edit::LineEdit::new());
    type_text(&mut s.app, "/definitely/not/here");
    press(&mut s.app, KeyCode::Enter);
    assert!(s.app.folder_prompt.is_some());
    assert_eq!(
        s.app.toasts.current().unwrap().text,
        "not a folder: /definitely/not/here"
    );
}

#[test]
fn capital_f_opens_the_prompt_and_capital_u_rescans() {
    let mut s = setup();
    let music = music(&s.root);
    s.service
        .apply(znicz_core::LibraryOp::AddFolder { path: music })
        .unwrap();
    wait(&s.service, 1);
    draw(&mut s.app);
    press(&mut s.app, KeyCode::Esc); // dismiss the first-run prompt
    press(&mut s.app, KeyCode::Char('F'));
    assert!(s.app.folder_prompt.is_some());
    press(&mut s.app, KeyCode::Esc);
    press(&mut s.app, KeyCode::Char('U'));
    assert_eq!(s.app.toasts.current().unwrap().text, "rescanning 1 folder");
    wait(&s.service, 2);
}

#[test]
fn u_without_folders_explains() {
    let mut s = setup();
    draw(&mut s.app);
    press(&mut s.app, KeyCode::Esc);
    press(&mut s.app, KeyCode::Char('U'));
    assert_eq!(
        s.app.toasts.current().unwrap().text,
        "no music folders yet — press F to add one"
    );
}

#[test]
fn the_folder_prompt_has_its_own_footer() {
    let mut s = setup();
    let screen = draw(&mut s.app);
    let footer = screen.lines().last().unwrap();
    assert!(
        footer.contains("Tab complete · Enter scan · Esc cancel"),
        "{footer}"
    );
}

#[test]
fn the_prompt_sits_on_top_when_the_library_has_music() {
    let mut s = setup();
    Library::open(&s.root.join("library.db"))
        .unwrap()
        .upsert_track_for_test(
            Path::new("/m/a.flac"),
            "So What",
            Some("Miles".into()),
            Some("Kind of Blue".into()),
            None,
        )
        .unwrap();
    s.app.library.reload_browse();
    s.app
        .on_key(KeyEvent::new(KeyCode::Char('F'), KeyModifiers::NONE));
    let screen = draw(&mut s.app);
    let row = screen.lines().next().unwrap_or_default();
    assert!(
        row.contains("folder: "),
        "prompt on the pane's first row:\n{screen}"
    );
    assert!(screen.contains("Miles"), "{screen}");
}

/// A host that serves queued answers, then repeats the last one.
struct Scripted {
    answers: std::sync::Mutex<Vec<Result<znicz_core::LibraryStatus, String>>>,
    calls: std::sync::atomic::AtomicUsize,
}

impl Scripted {
    fn new(answers: Vec<Result<znicz_core::LibraryStatus, String>>) -> Arc<Self> {
        Arc::new(Self {
            answers: std::sync::Mutex::new(answers),
            calls: std::sync::atomic::AtomicUsize::new(0),
        })
    }
}

impl LibraryHost for Scripted {
    fn apply(&self, _op: znicz_core::LibraryOp) -> znicz_core::Result<()> {
        Ok(())
    }
    fn status(&self) -> znicz_core::LibraryStatus {
        self.try_status().unwrap_or_default()
    }
    fn try_status(&self) -> znicz_core::Result<znicz_core::LibraryStatus> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut answers = self.answers.lock().unwrap();
        let next = if answers.len() > 1 {
            answers.remove(0)
        } else {
            answers[0].clone()
        };
        next.map_err(znicz_core::ZniczError::Library)
    }
}

fn status(finished: u64, running: bool) -> znicz_core::LibraryStatus {
    znicz_core::LibraryStatus {
        scans_finished: finished,
        running,
        ..Default::default()
    }
}

#[test]
fn an_unreachable_host_keeps_the_last_status() {
    let mut s = setup();
    let host = Scripted::new(vec![Ok(status(5, true)), Err("connection lost".into())]);
    s.app.library_host = Some(host);
    s.app.poll_library();
    assert_eq!(s.app.library_status.scans_finished, 5);
    s.app.poll_library();
    assert_eq!(s.app.library_status.scans_finished, 5);
    assert!(s.app.library_status.busy());
    assert!(s.app.folder_prompt.is_none());
    assert!(s.app.toasts.current().is_none());
}

#[test]
fn polling_is_throttled() {
    let mut s = setup();
    let host = Scripted::new(vec![Ok(status(1, false))]);
    s.app.library_host = Some(host.clone());
    s.app.library_poll_every = Duration::from_secs(60);
    for _ in 0..5 {
        s.app.poll_library();
    }
    assert_eq!(host.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn a_finished_scan_does_not_pull_the_user_out_of_search() {
    let mut s = setup();
    Library::open(&s.root.join("library.db"))
        .unwrap()
        .upsert_track_for_test(
            Path::new("/m/a.flac"),
            "So What",
            Some("Miles".into()),
            None,
            None,
        )
        .unwrap();
    s.app.library.reload_browse();
    s.app.library.begin_search();
    let host = Scripted::new(vec![Ok(status(0, false)), Ok(status(1, false))]);
    s.app.library_host = Some(host);
    s.app.poll_library();
    s.app.poll_library();
    assert!(s.app.library.is_typing(), "still searching");
}
