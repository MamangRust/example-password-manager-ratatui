use std::{
    error::Error,
    fs::{File, OpenOptions},
    io::{self, BufRead, BufReader, Write},
    path::Path,
    time::Duration,
};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Terminal,
};

#[derive(Debug, Clone)]
struct Entry {
    account: String,
    password: String,
}

enum InputMode {
    Normal,
    EditingAccount,
    EditingPassword,
}

struct App {
    entries: Vec<Entry>,
    selected: usize,
    list_state: ratatui::widgets::ListState,
    input_mode: InputMode,
    account_input: String,
    password_input: String,
    message: Option<String>,
}

impl App {
    fn new(entries: Vec<Entry>) -> App {
        let mut list_state = ratatui::widgets::ListState::default();
        if !entries.is_empty() {
            list_state.select(Some(0));
        }
        App {
            entries,
            selected: 0,
            list_state,
            input_mode: InputMode::Normal,
            account_input: String::new(),
            password_input: String::new(),
            message: None,
        }
    }

    fn next(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        if self.selected < self.entries.len() - 1 {
            self.selected += 1;
        } else {
            self.selected = 0;
        }
        self.list_state.select(Some(self.selected));
    }

    fn previous(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        if self.selected > 0 {
            self.selected -= 1;
        } else {
            self.selected = self.entries.len() - 1;
        }
        self.list_state.select(Some(self.selected));
    }

    fn add_entry(&mut self) {
        let new_entry = Entry {
            account: self.account_input.trim().to_string(),
            password: self.password_input.trim().to_string(),
        };
        if new_entry.account.is_empty() || new_entry.password.is_empty() {
            self.message = Some("Account atau password tidak boleh kosong.".to_string());
            return;
        }
        self.entries.push(new_entry);
        self.account_input.clear();
        self.password_input.clear();
        self.message = Some("Entri berhasil ditambahkan.".to_string());
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn load_entries(path: &str) -> io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    if !Path::new(path).exists() {
        return Ok(entries);
    }
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    for line in reader.lines() {
        let line = line?;
        let parts: Vec<&str> = line.splitn(2, ',').collect();
        if parts.len() == 2 {
            entries.push(Entry {
                account: parts[0].to_string(),
                password: parts[1].to_string(),
            });
        }
    }
    Ok(entries)
}

fn save_entries(path: &str, entries: &[Entry]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    for entry in entries {
        writeln!(file, "{},{}", entry.account, entry.password)?;
    }
    Ok(())
}

fn ui<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> io::Result<()> {
    terminal.draw(|f| {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(2)
            .constraints(
                [
                    Constraint::Min(5),
                    Constraint::Length(3),
                    Constraint::Length(3),
                ]
                .as_ref(),
            )
            .split(f.area());

        let items: Vec<ListItem> = app
            .entries
            .iter()
            .map(|entry| ListItem::new(Span::raw(&entry.account)))
            .collect();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Password Manager"))
            .highlight_style(
                Style::default()
                    .bg(Color::Blue)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(">> ");
        f.render_stateful_widget(list, chunks[0], &mut app.list_state);

        let msg = match &app.message {
            Some(m) => m.as_str(),
            None => "",
        };
        let message = Paragraph::new(msg)
            .block(Block::default().borders(Borders::ALL).title("Pesan"))
            .alignment(Alignment::Center);
        f.render_widget(message, chunks[1]);

    
        let instructions = match app.input_mode {
            InputMode::Normal => "Tekan 'a' untuk menambah entri, 'v' untuk melihat password, 'q' untuk keluar, panah Up/Down untuk navigasi",
            InputMode::EditingAccount => "Masukkan nama account: (Enter untuk lanjut, Esc untuk batal)",
            InputMode::EditingPassword => "Masukkan password: (Enter untuk simpan, Esc untuk batal)",
        };
        let instruction = Paragraph::new(instructions)
            .block(Block::default().borders(Borders::ALL).title("Instruksi"))
            .alignment(Alignment::Center);
        f.render_widget(instruction, chunks[2]);

        match app.input_mode {
            InputMode::EditingAccount | InputMode::EditingPassword => {
                let area = centered_rect(60, 20, f.area());
                f.render_widget(Clear, area);

                let title = if let InputMode::EditingAccount = app.input_mode {
                    "Entri Baru - Account"
                } else {
                    "Entri Baru - Password"
                };
                let input_text = if let InputMode::EditingAccount = app.input_mode {
                    &app.account_input
                } else {
                    &app.password_input
                };
                let input = Paragraph::new(input_text.clone())
                    .block(Block::default().borders(Borders::ALL).title(title))
                    .alignment(Alignment::Center);
                f.render_widget(input, area);
            }
            _ => {}
        }
    })?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let data_file = "passwords.txt";
    let entries = load_entries(data_file).unwrap_or_else(|err| {
        eprintln!("Error memuat entri: {}", err);
        Vec::new()
    });
    let mut app = App::new(entries);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    loop {
        ui(&mut terminal, &mut app)?;

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                match app.input_mode {
                    InputMode::Normal => match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Down => app.next(),
                        KeyCode::Up => app.previous(),
                        KeyCode::Char('a') => {
                            app.input_mode = InputMode::EditingAccount;
                        }
                        KeyCode::Char('v') => {
                            if let Some(entry) = app.entries.get(app.selected) {
                                app.message = Some(format!(
                                    "Password untuk {}: {}",
                                    entry.account, entry.password
                                ));
                            }
                        }
                        _ => {}
                    },
                    InputMode::EditingAccount => match key.code {
                        KeyCode::Esc => {
                            app.input_mode = InputMode::Normal;
                            app.account_input.clear();
                        }
                        KeyCode::Enter => {
                            app.input_mode = InputMode::EditingPassword;
                        }
                        KeyCode::Char(c) => {
                            app.account_input.push(c);
                        }
                        KeyCode::Backspace => {
                            app.account_input.pop();
                        }
                        _ => {}
                    },
                    InputMode::EditingPassword => match key.code {
                        KeyCode::Esc => {
                            app.input_mode = InputMode::Normal;
                            app.password_input.clear();
                        }
                        KeyCode::Enter => {
                            app.add_entry();
                            if let Err(e) = save_entries(data_file, &app.entries) {
                                app.message = Some(format!("Error menyimpan entri: {}", e));
                            }
                            app.input_mode = InputMode::Normal;
                        }
                        KeyCode::Char(c) => {
                            app.password_input.push(c);
                        }
                        KeyCode::Backspace => {
                            app.password_input.pop();
                        }
                        _ => {}
                    },
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}
