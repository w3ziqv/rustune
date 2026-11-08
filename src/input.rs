//! Bounded terminal input, including bracketed paste. No raw input logging.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use std::{
    collections::VecDeque,
    fs::File,
    io::{self, Read, Write},
    os::{fd::AsFd, unix::net::UnixStream},
};
use zeroize::{Zeroize, Zeroizing};
pub enum InputEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Paste(Zeroizing<String>),
    Overflow,
}
#[derive(Default)]
pub struct Parser {
    escape: Zeroizing<Vec<u8>>,
    utf8: Zeroizing<Vec<u8>>,
    paste: Zeroizing<Vec<u8>>,
    pasting: bool,
    closing: usize,
    overflow: bool,
}
fn key(code: KeyCode, modifiers: KeyModifiers) -> InputEvent {
    InputEvent::Key(KeyEvent::new(code, modifiers))
}
fn modifiers(n: u16) -> KeyModifiers {
    let n = n.saturating_sub(1);
    let mut m = KeyModifiers::NONE;
    if n & 1 != 0 {
        m |= KeyModifiers::SHIFT;
    }
    if n & 2 != 0 {
        m |= KeyModifiers::ALT;
    }
    if n & 4 != 0 {
        m |= KeyModifiers::CONTROL;
    }
    m
}
impl Parser {
    pub fn retained_bytes(&self) -> usize {
        self.escape.len() + self.utf8.len() + self.paste.len() + self.closing
    }
    fn append_paste(&mut self, byte: u8) {
        if !self.overflow {
            if self.paste.len() < 16 * 1024 {
                self.paste.push(byte);
            } else {
                self.overflow = true;
                self.paste = Zeroizing::new(Vec::new());
            }
        }
    }
    pub fn feed(&mut self, byte: u8) -> Option<InputEvent> {
        if byte == 3 {
            *self = Self::default();
            return Some(key(KeyCode::Char('c'), KeyModifiers::CONTROL));
        }
        if self.pasting {
            const END: &[u8] = b"\x1b[201~";
            if byte == 3 {
                self.pasting = false;
                self.paste = Zeroizing::new(Vec::new());
                self.closing = 0;
                return Some(key(KeyCode::Char('c'), KeyModifiers::CONTROL));
            }
            if byte == END[self.closing] {
                self.closing += 1;
                if self.closing == END.len() {
                    self.pasting = false;
                    self.closing = 0;
                    let result = if self.overflow {
                        InputEvent::Overflow
                    } else {
                        std::str::from_utf8(&self.paste)
                            .ok()
                            .map(|s| InputEvent::Paste(Zeroizing::new(s.to_owned())))
                            .unwrap_or(InputEvent::Overflow)
                    };
                    self.paste = Zeroizing::new(Vec::new());
                    self.overflow = false;
                    return Some(result);
                }
                return None;
            }
            for b in END.iter().take(self.closing) {
                self.append_paste(*b);
            }
            self.closing = 0;
            if byte == 27 {
                self.closing = 1;
            } else {
                self.append_paste(byte);
            }
            return None;
        }
        if !self.escape.is_empty() || byte == 27 {
            self.escape.push(byte);
            if self.escape.len() > 64 {
                self.escape = Zeroizing::new(Vec::new());
                return None;
            }
            if self.escape.as_slice() == b"\x1b[200~" {
                self.escape = Zeroizing::new(Vec::new());
                self.pasting = true;
                self.overflow = false;
                return None;
            }
            let complete = if self.escape.len() < 2 {
                false
            } else if self.escape[1] == b'[' {
                self.escape.len() > 2 && matches!(byte, 0x40..=0x7e)
            } else if self.escape[1] == b'O' {
                self.escape.len() >= 3
            } else {
                std::str::from_utf8(&self.escape[1..]).is_ok()
            };
            if complete {
                let result = parse_escape(&self.escape);
                self.escape = Zeroizing::new(Vec::new());
                return result;
            }
            return None;
        }
        if byte >= 128 || !self.utf8.is_empty() {
            self.utf8.push(byte);
            if let Ok(s) = std::str::from_utf8(&self.utf8) {
                let c = s.chars().next();
                self.utf8 = Zeroizing::new(Vec::new());
                return c.map(|c| key(KeyCode::Char(c), KeyModifiers::NONE));
            }
            if self.utf8.len() >= 4 {
                self.utf8 = Zeroizing::new(Vec::new());
            }
            return None;
        }
        Some(match byte {
            b'\r' | b'\n' => key(KeyCode::Enter, KeyModifiers::NONE),
            b'\t' => key(KeyCode::Tab, KeyModifiers::NONE),
            8 | 127 => key(KeyCode::Backspace, KeyModifiers::NONE),
            0 => key(KeyCode::Char(' '), KeyModifiers::CONTROL),
            1..=26 => key(
                KeyCode::Char((b'a' + byte - 1) as char),
                KeyModifiers::CONTROL,
            ),
            _ => key(KeyCode::Char(byte as char), KeyModifiers::NONE),
        })
    }
    fn escape_pending(&self) -> bool {
        self.escape.len() == 1
    }
    fn flush_escape(&mut self) -> Option<InputEvent> {
        if self.escape.is_empty() {
            None
        } else {
            self.escape = Zeroizing::new(Vec::new());
            Some(key(KeyCode::Esc, KeyModifiers::NONE))
        }
    }
}
fn parse_escape(bytes: &[u8]) -> Option<InputEvent> {
    if bytes.len() < 2 {
        return None;
    }
    if !matches!(bytes[1], b'[' | b'O') {
        return std::str::from_utf8(&bytes[1..])
            .ok()?
            .chars()
            .next()
            .map(|c| key(KeyCode::Char(c), KeyModifiers::ALT));
    }
    let text = std::str::from_utf8(&bytes[2..]).ok()?;
    let final_byte = *bytes.last()?;
    if let Some(mouse) = text
        .strip_prefix('<')
        .and_then(|s| s.strip_suffix(['M', 'm']))
    {
        let values: Vec<u16> = mouse
            .split(';')
            .map(str::parse)
            .collect::<Result<_, _>>()
            .ok()?;
        if values.len() != 3 {
            return None;
        }
        let code = values[0];
        let button = match code & 3 {
            0 => MouseButton::Left,
            1 => MouseButton::Middle,
            _ => MouseButton::Right,
        };
        let kind = if code & 64 != 0 {
            if code & 1 == 0 {
                MouseEventKind::ScrollUp
            } else {
                MouseEventKind::ScrollDown
            }
        } else if final_byte == b'm' {
            MouseEventKind::Up(button)
        } else if code & 32 != 0 {
            MouseEventKind::Drag(button)
        } else {
            MouseEventKind::Down(button)
        };
        let mut mods = KeyModifiers::NONE;
        if code & 4 != 0 {
            mods |= KeyModifiers::SHIFT;
        }
        if code & 8 != 0 {
            mods |= KeyModifiers::ALT;
        }
        if code & 16 != 0 {
            mods |= KeyModifiers::CONTROL;
        }
        return Some(InputEvent::Mouse(MouseEvent {
            kind,
            column: values[1].saturating_sub(1),
            row: values[2].saturating_sub(1),
            modifiers: mods,
        }));
    }
    let params = &text[..text.len().saturating_sub(1)];
    let values: Vec<u16> = params
        .split(';')
        .filter(|s| !s.is_empty())
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    let mods = values.get(1).copied().map_or(KeyModifiers::NONE, modifiers);
    let code = match final_byte {
        b'A' => KeyCode::Up,
        b'B' => KeyCode::Down,
        b'C' => KeyCode::Right,
        b'D' => KeyCode::Left,
        b'H' => KeyCode::Home,
        b'F' => KeyCode::End,
        b'Z' => return Some(key(KeyCode::BackTab, KeyModifiers::SHIFT)),
        b'P' => KeyCode::F(1),
        b'Q' => KeyCode::F(2),
        b'R' => KeyCode::F(3),
        b'S' => KeyCode::F(4),
        b'~' => match values.first()? {
            1 | 7 => KeyCode::Home,
            4 | 8 => KeyCode::End,
            2 => KeyCode::Insert,
            3 => KeyCode::Delete,
            5 => KeyCode::PageUp,
            6 => KeyCode::PageDown,
            11 => KeyCode::F(1),
            12 => KeyCode::F(2),
            13 => KeyCode::F(3),
            14 => KeyCode::F(4),
            15 => KeyCode::F(5),
            17 => KeyCode::F(6),
            18 => KeyCode::F(7),
            19 => KeyCode::F(8),
            20 => KeyCode::F(9),
            21 => KeyCode::F(10),
            23 => KeyCode::F(11),
            24 => KeyCode::F(12),
            _ => return None,
        },
        _ => return None,
    };
    Some(key(code, mods))
}

pub struct Reader {
    file: File,
    cancel: Option<UnixStream>,
    parser: Parser,
    events: VecDeque<InputEvent>,
}
impl Reader {
    pub fn new() -> io::Result<Self> {
        Self::with_cancel(None)
    }
    fn with_cancel(cancel: Option<UnixStream>) -> io::Result<Self> {
        Ok(Self {
            file: File::from(io::stdin().as_fd().try_clone_to_owned()?),
            cancel,
            parser: Parser::default(),
            events: VecDeque::new(),
        })
    }
    pub fn read_event(&mut self) -> io::Result<Option<InputEvent>> {
        loop {
            if let Some(event) = self.events.pop_front() {
                return Ok(Some(event));
            }
            let mut fds = vec![PollFd::new(self.file.as_fd(), PollFlags::POLLIN)];
            if let Some(cancel) = &self.cancel {
                fds.push(PollFd::new(cancel.as_fd(), PollFlags::POLLIN));
            }
            let timeout = if self.parser.escape_pending() {
                PollTimeout::from(250u16)
            } else {
                PollTimeout::NONE
            };
            let count = match poll(&mut fds, timeout) {
                Ok(n) => n,
                Err(nix::errno::Errno::EINTR) => continue,
                Err(e) => return Err(io::Error::from_raw_os_error(e as i32)),
            };
            if count == 0 {
                if let Some(event) = self.parser.flush_escape() {
                    return Ok(Some(event));
                }
                continue;
            }
            if fds.get(1).is_some_and(|p| {
                p.revents()
                    .is_some_and(|r| r.intersects(PollFlags::POLLIN | PollFlags::POLLHUP))
            }) {
                return Ok(None);
            }
            let mut bytes = Zeroizing::new([0u8; 512]);
            let n = self.file.read(&mut *bytes)?;
            if n == 0 {
                return Ok(None);
            }
            for byte in &bytes[..n] {
                if let Some(event) = self.parser.feed(*byte) {
                    self.events.push_back(event);
                }
            }
            bytes.zeroize();
        }
    }
}
pub struct Events {
    rx: tokio::sync::mpsc::Receiver<io::Result<InputEvent>>,
    wake: UnixStream,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Events {
    pub fn new() -> io::Result<Self> {
        let (wake, cancel) = UnixStream::pair()?;
        let mut reader = Reader::with_cancel(Some(cancel))?;
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let thread = std::thread::Builder::new()
            .name("rustune-input".into())
            .spawn(move || {
                loop {
                    match reader.read_event() {
                        Ok(Some(e)) => {
                            if tx.blocking_send(Ok(e)).is_err() {
                                break;
                            }
                        }
                        Ok(None) => break,
                        Err(e) => {
                            let _ = tx.blocking_send(Err(e));
                            break;
                        }
                    }
                }
            })?;
        Ok(Self {
            rx,
            wake,
            thread: Some(thread),
        })
    }
    pub async fn next(&mut self) -> Option<io::Result<InputEvent>> {
        self.rx.recv().await
    }
}
impl Drop for Events {
    fn drop(&mut self) {
        self.rx.close();
        let _ = self.wake.write_all(&[1]);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
