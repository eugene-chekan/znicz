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
}

fn setup(name: &str) -> Setup {
    let root = std::env::temp_dir().join(format!(
        "znicz-tui-scan-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let root = std::fs::canonicalize(&root).unwrap();
    let db = root.join("library.db");
    let service = Arc::new(LibraryService::open(&db).unwrap());
    let (player, _thread) = spawn_player(AudioConfig::default());
    let mut app = App::with_library(player, Some(Library::open(&db).unwrap()));
    app.library_host = Some(service.clone());
    Setup { app, root, service }
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
#[ignore = "drawn in Task 10"]
fn an_empty_library_opens_the_folder_prompt() {
    let mut s = setup("first-run");
    let screen = draw(&mut s.app);
    assert!(s.app.folder_prompt.is_some());
    assert!(
        screen.contains("No music yet. Which folder holds your music?"),
        "{screen}"
    );
    assert!(screen.contains("folder: "), "{screen}");
}

#[test]
fn esc_skips_the_prompt_for_the_session() {
    let mut s = setup("skip");
    draw(&mut s.app);
    press(&mut s.app, KeyCode::Esc);
    let screen = draw(&mut s.app);
    assert!(s.app.folder_prompt.is_none(), "does not reopen");
    assert!(screen.contains("press F to add a folder"), "{screen}");
}

#[test]
fn enter_adds_the_folder_and_the_library_reloads_when_done() {
    let mut s = setup("add");
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
    let mut s = setup("bad");
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
    let mut s = setup("keys");
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
    let mut s = setup("no-folders");
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
    let mut s = setup("footer");
    let screen = draw(&mut s.app);
    let footer = screen.lines().last().unwrap();
    assert!(
        footer.contains("Tab complete · Enter scan · Esc cancel"),
        "{footer}"
    );
}
