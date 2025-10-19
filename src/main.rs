use std::{
    env,
    error::Error,
    fs::{File, OpenOptions},
    io::{self, BufRead, BufReader, Write},
    path::Path,
    time::Duration,
};
use dotenv::dotenv;

use aes_gcm::{aead::Aead, aead::KeyInit, Aes256Gcm, Nonce};
use base64::{engine::general_purpose, Engine as _};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use rand::RngCore;
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Terminal,
};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
struct Entry {
    account: String,
    password: String,
}

#[derive(Debug, Clone, Copy)]
enum FeedbackKind {
    Info,
    Success,
    Error,
}

#[derive(Debug, Clone)]
struct Feedback {
    text: String,
    kind: FeedbackKind,
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
    feedback: Option<Feedback>,
    cipher: Aes256Gcm,
}

fn initialize_cipher() -> Result<Aes256Gcm, String> {
    let passphrase = env::var("PASSWORD_MANAGER_KEY")
        .map_err(|_| "Environment variable PASSWORD_MANAGER_KEY belum diset.".to_string())?;
    if passphrase.trim().is_empty() {
        return Err("PASSWORD_MANAGER_KEY tidak boleh kosong.".to_string());
    }
    let digest = Sha256::digest(passphrase.as_bytes());
    Aes256Gcm::new_from_slice(&digest).map_err(|_| "Gagal menginisialisasi cipher.".to_string())
}

fn encrypt_password(cipher: &Aes256Gcm, plaintext: &str) -> Result<String, String> {
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from(nonce_bytes);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|e| format!("Gagal mengenkripsi password: {}", e))?;
    let encoded_nonce = general_purpose::STANDARD.encode(nonce_bytes);
    let encoded_cipher = general_purpose::STANDARD.encode(ciphertext);
    Ok(format!("{}:{}", encoded_nonce, encoded_cipher))
}

fn decode_encrypted_components(value: &str) -> Result<(Vec<u8>, Vec<u8>), String> {
    let (nonce_b64, cipher_b64) = value
        .split_once(':')
        .ok_or_else(|| "Format data enkripsi tidak valid.".to_string())?;
    let nonce_bytes = general_purpose::STANDARD
        .decode(nonce_b64)
        .map_err(|_| "Nonce terenkripsi tidak valid.".to_string())?;
    if nonce_bytes.len() != 12 {
        return Err("Panjang nonce tidak valid.".to_string());
    }
    let cipher_bytes = general_purpose::STANDARD
        .decode(cipher_b64)
        .map_err(|_| "Ciphertext terenkripsi tidak valid.".to_string())?;
    Ok((nonce_bytes, cipher_bytes))
}

fn decrypt_password(cipher: &Aes256Gcm, value: &str) -> Result<String, String> {
    let (nonce_bytes, cipher_bytes) = decode_encrypted_components(value)?;
    let nonce_array: [u8; 12] = nonce_bytes
        .as_slice()
        .try_into()
        .map_err(|_| "Nonce terenkripsi tidak valid.".to_string())?;
    let nonce = Nonce::from(nonce_array);
    let plaintext = cipher
        .decrypt(&nonce, cipher_bytes.as_ref())
        .map_err(|_| "Gagal mendekripsi password.".to_string())?;
    String::from_utf8(plaintext).map_err(|_| "Password terdekripsi bukan UTF-8 valid.".to_string())
}

impl App {
    fn new(entries: Vec<Entry>, cipher: Aes256Gcm) -> App {
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
            feedback: None,
            cipher,
        }
    }

    fn set_feedback(&mut self, text: impl Into<String>, kind: FeedbackKind) {
        self.feedback = Some(Feedback {
            text: text.into(),
            kind,
        });
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

    fn add_entry(&mut self) -> Result<(), String> {
        let new_entry = Entry {
            account: self.account_input.trim().to_string(),
            password: self.password_input.trim().to_string(),
        };
        if new_entry.account.is_empty() || new_entry.password.is_empty() {
            return Err("Account atau password tidak boleh kosong.".to_string());
        }
        let encrypted = encrypt_password(&self.cipher, &new_entry.password)?;
        self.entries.push(Entry {
            account: new_entry.account,
            password: encrypted,
        });
        self.account_input.clear();
        self.password_input.clear();
        Ok(())
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

fn is_encrypted_format(value: &str) -> bool {
    value.contains(':')
        && value
            .split_once(':')
            .map(|(n, c)| n.len() > 0 && c.len() > 0)
            .unwrap_or(false)
}

fn load_entries(path: &str, cipher: &Aes256Gcm) -> io::Result<(Vec<Entry>, bool)> {
    let mut entries = Vec::new();
    let mut updated = false;
    if !Path::new(path).exists() {
        return Ok((entries, updated));
    }
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    for line in reader.lines() {
        let line = line?;
        let parts: Vec<&str> = line.splitn(2, ',').collect();
        if parts.len() == 2 {
            let account = parts[0].to_string();
            let raw_password = parts[1].to_string();
            let password = if is_encrypted_format(&raw_password) {
                raw_password
            } else {
                updated = true;
                encrypt_password(cipher, &raw_password)
                    .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{}", e)))?
            };
            entries.push(Entry { account, password });
        }
    }
    Ok((entries, updated))
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
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Min(8),
                    Constraint::Length(4),
                ]
                .as_ref(),
            )
            .split(f.area());

        let status_text = format!(
            "Total Entri: {} | Mode: {}",
            app.entries.len(),
            match app.input_mode {
                InputMode::Normal => "Normal",
                InputMode::EditingAccount => "Input Account",
                InputMode::EditingPassword => "Input Password",
            }
        );
        let status = Paragraph::new(status_text)
            .style(Style::default().fg(Color::White))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Password Manager - Status"),
            );
        f.render_widget(status, chunks[0]);

        let (feedback_text, feedback_style) = match &app.feedback {
            Some(feedback) => {
                let color = match feedback.kind {
                    FeedbackKind::Info => Color::Cyan,
                    FeedbackKind::Success => Color::Green,
                    FeedbackKind::Error => Color::Red,
                };
                (
                    feedback.text.clone(),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                )
            }
            None => (
                "Gunakan panah atas/bawah untuk navigasi, tekan 'a' untuk menambah entri."
                    .to_string(),
                Style::default().fg(Color::DarkGray),
            ),
        };
        let feedback = Paragraph::new(feedback_text)
            .style(feedback_style)
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL).title("Notifikasi"));
        f.render_widget(feedback, chunks[1]);

        let main_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)].as_ref())
            .split(chunks[2]);

        let items: Vec<ListItem> = app
            .entries
            .iter()
            .map(|entry| {
                ListItem::new(entry.account.clone()).style(Style::default().fg(Color::White))
            })
            .collect();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Daftar Akun"))
            .highlight_style(
                Style::default()
                    .bg(Color::Blue)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(">> ");
        f.render_stateful_widget(list, main_chunks[0], &mut app.list_state);

        let detail_block = Block::default().borders(Borders::ALL).title("Detail Akun");

        let detail_text = if let Some(entry) = app.entries.get(app.selected) {
            let masked_password = "*".repeat(entry.password.len().min(32).max(1));
            Text::from(vec![
                Line::from(Span::styled(
                    format!("Akun: {}", entry.account),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::default(),
                Line::from(format!(
                    "Password terenkripsi (disembunyikan): {}",
                    masked_password
                )),
                Line::from("Tekan 'v' untuk melihat password asli pada notifikasi."),
            ])
        } else {
            Text::from(vec![
                Line::from("Belum ada entri."),
                Line::from("Tekan 'a' untuk menambahkan akun baru."),
            ])
        };
        let detail = Paragraph::new(detail_text)
            .block(detail_block)
            .alignment(Alignment::Left);
        f.render_widget(detail, main_chunks[1]);

        let instruction_lines = match app.input_mode {
            InputMode::Normal => vec![
                "[Navigasi] Panah Atas/Bawah",
                "[Tambah] 'a'",
                "[Lihat Password] 'v'",
                "[Keluar] 'q'",
            ],
            InputMode::EditingAccount => vec![
                "Masukkan nama akun.",
                "Enter untuk lanjut ke input password.",
                "Esc untuk membatalkan penambahan.",
            ],
            InputMode::EditingPassword => vec![
                "Masukkan password.",
                "Enter untuk menyimpan entri.",
                "Esc untuk membatalkan penambahan.",
            ],
        };

        let instruction_text = Text::from(
            instruction_lines
                .into_iter()
                .map(Line::from)
                .collect::<Vec<_>>(),
        );

        let instruction = Paragraph::new(instruction_text)
            .block(Block::default().borders(Borders::ALL).title("Instruksi"))
            .alignment(Alignment::Left);
        f.render_widget(instruction, chunks[3]);

        match app.input_mode {
            InputMode::EditingAccount | InputMode::EditingPassword => {
                let area = centered_rect(60, 20, f.area());
                f.render_widget(Clear, area);

                let title = if let InputMode::EditingAccount = app.input_mode {
                    "Entri Baru - Account"
                } else {
                    "Entri Baru - Password"
                };
                let (input_text, counter) = if let InputMode::EditingAccount = app.input_mode {
                    (&app.account_input, app.account_input.chars().count())
                } else {
                    (&app.password_input, app.password_input.chars().count())
                };
                let popup_text = Text::from(vec![
                    Line::from(input_text.clone()),
                    Line::from(Span::styled(
                        format!("Karakter: {}", counter),
                        Style::default().fg(Color::DarkGray),
                    )),
                ]);
                let input = Paragraph::new(popup_text)
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
    dotenv().ok();    

    let data_file = "passwords.txt";
    let cipher = initialize_cipher()?;
    let (entries, mutated) = load_entries(data_file, &cipher).unwrap_or_else(|err| {
        eprintln!("Error memuat entri: {}", err);
        (Vec::new(), false)
    });
    let mut app = App::new(entries, cipher);
    if mutated {
        if let Err(err) = save_entries(data_file, &app.entries) {
            eprintln!("Error menyimpan ulang entri terenkripsi: {}", err);
        }
    }

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
                                match decrypt_password(&app.cipher, &entry.password) {
                                    Ok(plain) => {
                                        app.set_feedback(
                                            format!("Password untuk {}: {}", entry.account, plain),
                                            FeedbackKind::Info,
                                        );
                                    }
                                    Err(err) => {
                                        app.set_feedback(err, FeedbackKind::Error);
                                    }
                                }
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
                        KeyCode::Enter => match app.add_entry() {
                            Ok(_) => {
                                if let Err(e) = save_entries(data_file, &app.entries) {
                                    app.set_feedback(
                                        format!("Error menyimpan entri: {}", e),
                                        FeedbackKind::Error,
                                    );
                                } else {
                                    app.set_feedback(
                                        "Entri berhasil ditambahkan dan password terenkripsi.",
                                        FeedbackKind::Success,
                                    );
                                }
                                app.input_mode = InputMode::Normal;
                            }
                            Err(msg) => {
                                app.set_feedback(msg, FeedbackKind::Error);
                            }
                        },
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
