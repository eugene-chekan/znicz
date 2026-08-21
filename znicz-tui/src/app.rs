use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, Paragraph};
use ratatui::Frame;

use znicz_core::{Command, PlaybackStatus, PlayerEvent, PlayerHandle, PlayerState};

const TICK_RATE: Duration = Duration::from_millis(100);

pub struct App {
    pub player: PlayerHandle,
    pub should_quit: bool,
    pub show_help: bool,
}

impl App {
    pub fn new(player: PlayerHandle) -> Self {
        Self {
            player,
            should_quit: false,
            show_help: false,
        }
    }

    pub fn run(&mut self) -> color_eyre::Result<()> {
        let mut terminal = ratatui::init();
        let result = self.run_loop(&mut terminal);
        ratatui::restore();
        result
    }

    fn run_loop(&mut self, terminal: &mut ratatui::DefaultTerminal) -> color_eyre::Result<()> {
        let mut last_tick = std::time::Instant::now();

        loop {
            self.poll_player_events();
            terminal.draw(|frame| ui::render(frame, self))?;

            let timeout = TICK_RATE.saturating_sub(last_tick.elapsed());
            if event::poll(timeout)? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        self.on_key(key.code);
                    }
                }
            } else {
                last_tick = std::time::Instant::now();
            }

            if self.should_quit {
                break;
            }
        }
        Ok(())
    }

    fn poll_player_events(&mut self) {
        for event in self.player.drain_events() {
            if let PlayerEvent::Error(msg) = event {
                tracing::error!("player error: {msg}");
            }
        }
    }

    fn on_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.show_help = !self.show_help,
            KeyCode::Char(' ') | KeyCode::Enter => self.toggle_pause(),
            KeyCode::Char('n') => {
                self.player.send(Command::NextTrack).ok();
            }
            KeyCode::Char('p') => {
                self.player.send(Command::PreviousTrack).ok();
            }
            KeyCode::Char('+') | KeyCode::Char('=') => self.adjust_volume(0.05),
            KeyCode::Char('-') | KeyCode::Char('_') => self.adjust_volume(-0.05),
            KeyCode::Right => self.seek_relative(5),
            KeyCode::Left => self.seek_relative(-5),
            _ => {}
        }
    }

    fn toggle_pause(&mut self) {
        let status = self.player.state().status;
        match status {
            PlaybackStatus::Playing => {
                self.player.send(Command::Pause).ok();
            }
            PlaybackStatus::Paused => {
                self.player.send(Command::Resume).ok();
            }
            PlaybackStatus::Stopped => {}
        }
    }

    fn adjust_volume(&mut self, delta: f32) {
        let vol = (self.player.state().volume + delta).clamp(0.0, 1.0);
        self.player.send(Command::SetVolume(vol)).ok();
    }

    fn seek_relative(&mut self, seconds: i64) {
        let state = self.player.state();
        let new_secs = state.position.as_secs() as i64 + seconds;
        let pos = Duration::from_secs(new_secs.max(0) as u64);
        self.player.send(Command::Seek(pos)).ok();
    }

    pub fn state(&self) -> PlayerState {
        self.player.state()
    }
}

mod ui {
    use super::*;

    pub fn render(frame: &mut Frame, app: &App) {
        let state = app.state();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(6),
                Constraint::Length(8),
                Constraint::Length(3),
            ])
            .split(frame.area());

        render_now_playing(frame, chunks[0], &state);
        render_queue(frame, chunks[1], &state);
        render_status_bar(frame, chunks[2], &state);

        if app.show_help {
            render_help(frame);
        }
    }

    fn render_now_playing(frame: &mut Frame, area: Rect, state: &PlayerState) {
        let track = state
            .current_track
            .as_ref()
            .map(|t| t.title.clone())
            .unwrap_or_else(|| "No track".to_string());

        let format = state
            .current_track
            .as_ref()
            .map(|t| t.format_description())
            .unwrap_or_else(|| "—".to_string());

        let total = state
            .current_track
            .as_ref()
            .and_then(|t| t.duration)
            .unwrap_or(Duration::ZERO);

        let progress = if total.is_zero() {
            0.0
        } else {
            state.position.as_secs_f64() / total.as_secs_f64()
        };

        let inner = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(1),
                Constraint::Length(3),
            ])
            .margin(1)
            .split(area);

        let title = Paragraph::new(Line::from(vec![
            Span::styled(
                track,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]))
        .block(Block::default().borders(Borders::ALL).title("Now Playing"));

        let gauge = Gauge::default()
            .block(Block::default().borders(Borders::LEFT | Borders::RIGHT))
            .gauge_style(Style::default().fg(Color::Green))
            .ratio(progress.clamp(0.0, 1.0))
            .label(format!(
                "{} / {}",
                format_duration(state.position),
                format_duration(total)
            ));

        let meta = Paragraph::new(format)
            .block(Block::default().borders(Borders::ALL).title("Format"));

        frame.render_widget(title, inner[0]);
        frame.render_widget(gauge, inner[1]);
        frame.render_widget(meta, inner[2]);
    }

    fn render_queue(frame: &mut Frame, area: Rect, state: &PlayerState) {
        let items: Vec<ListItem> = state
            .queue
            .iter()
            .enumerate()
            .map(|(i, path)| {
                let marker = if i == state.queue_position { "▶ " } else { "  " };
                let name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("?");
                ListItem::new(format!("{marker}{name}"))
            })
            .collect();

        let list = List::new(items).block(Block::default().borders(Borders::ALL).title("Queue"));
        frame.render_widget(list, area);
    }

    fn render_status_bar(frame: &mut Frame, area: Rect, state: &PlayerState) {
        let status = match state.status {
            PlaybackStatus::Playing => "Playing",
            PlaybackStatus::Paused => "Paused",
            PlaybackStatus::Stopped => "Stopped",
        };

        let device = state
            .device_name
            .as_deref()
            .or(state.device_id.as_deref())
            .unwrap_or("default");

        let text = format!(
            "{} | vol {:.0}% | {} | Space pause | n/p next/prev | ←/→ seek | ? help | q quit",
            status,
            state.volume * 100.0,
            device
        );

        let bar = Paragraph::new(text).style(Style::default().fg(Color::DarkGray));
        frame.render_widget(bar, area);
    }

    fn render_help(frame: &mut Frame) {
        let area = centered_rect(60, 50, frame.area());
        let help = Paragraph::new(
            "Keybindings:\n\
             Space     Play/Pause\n\
             n / →     Next track\n\
             p / ←     Previous track\n\
             + / -     Volume\n\
             ← / →     Seek ±5s (with Shift for track nav: use n/p)\n\
             ?         Toggle help\n\
             q         Quit",
        )
        .block(
            Block::default()
                .title("Help")
                .borders(Borders::ALL)
                .style(Style::default().bg(Color::Black)),
        );
        frame.render_widget(help, area);
    }

    fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
        let popup_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage((100 - percent_y) / 2),
                Constraint::Percentage(percent_y),
                Constraint::Percentage((100 - percent_y) / 2),
            ])
            .split(area);

        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage((100 - percent_x) / 2),
                Constraint::Percentage(percent_x),
                Constraint::Percentage((100 - percent_x) / 2),
            ])
            .split(popup_layout[1])[1]
    }

    fn format_duration(d: Duration) -> String {
        let secs = d.as_secs();
        format!("{:02}:{:02}", secs / 60, secs % 60)
    }
}
