//! Local user-token entry. Storage never implies remote acceptance or playback.
use crate::{
    config::{Paths, private_write},
    input::{InputEvent as Event, Reader},
};
use anyhow::{Result, ensure};
use crossterm::{
    event::{DisableBracketedPaste, EnableBracketedPaste, KeyCode, KeyModifiers},
    execute, terminal,
};
use std::{
    fmt,
    io::{self, IsTerminal, Write},
};
use zeroize::Zeroizing;
const MAX_TOKEN: usize = 16 * 1024;

pub struct SecretToken(Zeroizing<String>);
impl fmt::Debug for SecretToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretToken([redacted])")
    }
}
impl SecretToken {
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        ensure!(
            !s.is_empty() && s.len() <= MAX_TOKEN && s.bytes().all(|b| b.is_ascii_graphic()),
            "enter one nonempty Music User Token, at most 16 KiB, without internal whitespace"
        );
        Ok(Self(Zeroizing::new(s.to_owned())))
    }
}
pub fn save_user_token(paths: &Paths, token: &SecretToken) -> Result<()> {
    private_write(&paths.config.join("music-user-token"), token.0.as_bytes())
}
pub fn has_user_token(paths: &Paths) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(paths.config.join("music-user-token")).is_ok_and(|m| {
        m.is_file()
            && m.mode() & 0o077 == 0
            && m.uid() == nix::unistd::Uid::current().as_raw()
            && m.len() > 0
            && m.len() <= MAX_TOKEN as u64
    })
}
pub fn remove_user_token(paths: &Paths) -> Result<()> {
    let file = paths.config.join("music-user-token");
    match std::fs::symlink_metadata(&file) {
        Ok(m) => {
            ensure!(
                m.is_file() && !m.file_type().is_symlink(),
                "token path must be a regular file"
            );
            std::fs::remove_file(file)?;
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    Ok(())
}
#[derive(Default)]
pub struct TokenInput {
    text: Zeroizing<String>,
    overflow: bool,
}
impl TokenInput {
    pub fn append(&mut self, s: &str) {
        if self.text.len().saturating_add(s.len()) > MAX_TOKEN {
            self.overflow = true;
        } else {
            self.text.push_str(s);
        }
    }
    pub fn backspace(&mut self) {
        self.text.pop();
    }
    pub fn clear(&mut self) {
        self.text = Zeroizing::new(String::new());
        self.overflow = false;
    }
    pub fn reject_paste(&mut self) {
        self.text = Zeroizing::new(String::new());
        self.overflow = true;
    }
    pub fn feedback(&self, max_dots: usize) -> String {
        if self.overflow {
            return "Input too long; Ctrl-U clears it".into();
        }
        let count = self.text.chars().count();
        if count == 0 {
            "No token entered".into()
        } else {
            format!(
                "{}  ({count} characters)",
                "•".repeat(count.min(max_dots.min(32)))
            )
        }
    }
    pub fn finish(self) -> Result<SecretToken> {
        ensure!(
            !self.overflow,
            "token input exceeds 16 KiB; clear it and paste again"
        );
        SecretToken::parse(&self.text)
    }
}
pub enum LocalAuth {
    Save(SecretToken),
    Remove,
}
pub fn apply(paths: &Paths, job: LocalAuth) -> Result<()> {
    match job {
        LocalAuth::Save(token) => save_user_token(paths, &token),
        LocalAuth::Remove => remove_user_token(paths),
    }
}

pub const SAVED_MESSAGE: &str = "Token saved locally (0600). Apple account acceptance is not verified. Native playback is unavailable.";

struct RawInput;
fn feedback_line(input: &TokenInput, width: u16) -> String {
    let width = if width == 0 { 80 } else { width };
    let limit = usize::from(width.saturating_sub(1));
    let prefix = if limit >= 16 { "Token: " } else { "> " };
    let text = if input.overflow {
        "Too long; Ctrl-U clears".into()
    } else if input.text.is_empty() {
        "No token entered".into()
    } else {
        let count = input.text.chars().count();
        let suffix = format!(" ({count})");
        let dots = limit
            .saturating_sub(prefix.len() + suffix.len())
            .min(32)
            .min(count);
        format!("{}{suffix}", "•".repeat(dots))
    };
    format!("{prefix}{text}").chars().take(limit).collect()
}
impl Drop for RawInput {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), DisableBracketedPaste);
        let _ = terminal::disable_raw_mode();
    }
}
pub fn prompt_login(paths: &Paths) -> Result<()> {
    ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "login needs an interactive terminal"
    );
    terminal::enable_raw_mode()?;
    let guard = RawInput;
    execute!(io::stdout(), EnableBracketedPaste)?;
    let mut reader = Reader::new()?;
    print!("Music User Token (hidden)\r\nEnter saves · Esc cancels · Ctrl-U clears\r\n");
    let mut input = TokenInput::default();
    print!("{}", feedback_line(&input, terminal::size()?.0));
    io::stdout().flush()?;
    let token = loop {
        let Some(event) = reader.read_event()? else {
            break None;
        };
        match event {
            Event::Key(k) if k.is_press() => match k.code {
                KeyCode::Esc => break None,
                KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => break None,
                KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => input.clear(),
                KeyCode::Enter => break Some(input.finish()?),
                KeyCode::Backspace => input.backspace(),
                KeyCode::Char(c)
                    if !k
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    input.append(&c.to_string())
                }
                _ => {}
            },
            Event::Paste(text) => {
                input.append(&text);
            }
            Event::Overflow => input.reject_paste(),
            _ => {}
        }
        execute!(
            io::stdout(),
            crossterm::cursor::MoveToColumn(0),
            terminal::Clear(terminal::ClearType::CurrentLine)
        )?;
        print!("{}", feedback_line(&input, terminal::size()?.0));
        io::stdout().flush()?;
    };
    drop(guard);
    println!();
    if let Some(token) = token {
        save_user_token(paths, &token)?;
        println!("{SAVED_MESSAGE}");
    } else {
        println!("Login cancelled; saved token unchanged.");
    }
    Ok(())
}
