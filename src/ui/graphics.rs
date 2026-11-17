//! One bounded terminal probe, before the input reader is started.
//! No detached reader may survive a timeout and consume keyboard input.
use nix::poll::{PollFd, PollFlags, poll};
use ratatui_image::picker::cap_parser::{Parser, Response};
use std::{
    io::{Read, Write},
    os::fd::AsFd,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

#[derive(Default)]
pub(super) struct Graphics {
    pub protocol: Option<&'static str>,
    pub cell: (u16, u16),
    pub image_id: u32,
}
impl Graphics {
    fn response(&mut self, response: Response) -> bool {
        match response {
            Response::Kitty => self.protocol = Some("kitty"),
            Response::CellSize(Some((w, h))) if (1..=64).contains(&w) && (1..=128).contains(&h) => {
                self.cell = (w, h)
            }
            Response::Status => return true,
            _ => {}
        }
        false
    }
    pub fn detect() -> (Self, Zeroizing<Vec<u8>>) {
        let mut graphics = Self {
            image_id: fastrand::u32(1..=u32::MAX),
            ..Self::default()
        };
        let mut keys = Zeroizing::new(Vec::new());
        // Multiplexer passthrough is deliberately not advertised without verification.
        if std::env::var_os("TMUX").is_some() || std::env::var_os("STY").is_some() {
            return (graphics, keys);
        }
        let mut output = std::io::stdout();
        if output
            .write_all(b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[16t\x1b[5n")
            .and_then(|_| output.flush())
            .is_err()
        {
            return (graphics, keys);
        }
        let Ok(fd) = std::io::stdin().as_fd().try_clone_to_owned() else {
            return (graphics, keys);
        };
        // Stdin is buffered: a one-byte read may drain the kernel FD into its
        // private buffer, making the next poll time out despite pending replies.
        let mut input = std::fs::File::from(fd);
        let deadline = Instant::now() + Duration::from_millis(150);
        let mut parser = Parser::new();
        let mut sequence = Zeroizing::new(Vec::new());
        let mut total = 0;
        let mut done = false;
        while !done && Instant::now() < deadline && total < 4096 {
            let mut fds = [PollFd::new(input.as_fd(), PollFlags::POLLIN)];
            let timeout = deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
                .min(150) as u16;
            if !matches!(poll(&mut fds, timeout), Ok(n) if n > 0) {
                break;
            }
            let mut byte = [0u8];
            if !matches!(input.read(&mut byte), Ok(1)) {
                break;
            }
            total += 1;
            let b = byte[0];
            if sequence.is_empty() && b != 27 {
                keys.push(b);
                continue;
            }
            sequence.push(b);
            let responses = parser.push(char::from(b));
            if !responses.is_empty() {
                for response in responses {
                    done |= graphics.response(response);
                }
                sequence.clear();
            } else if sequence.len() >= 128
                || (sequence.len() >= 3
                    && (b.is_ascii_alphabetic() || b == b'~')
                    && sequence[1] == b'['
                    && b != b't'
                    && b != b'n')
            {
                keys.extend_from_slice(&sequence);
                sequence.clear();
                parser = Parser::new();
            }
        }
        keys.extend_from_slice(&sequence);
        if graphics.cell == (0, 0) {
            graphics.protocol = None;
        }
        if graphics.protocol.is_none()
            && graphics.cell != (0, 0)
            && (std::env::var("TERM_PROGRAM").is_ok_and(|s| s == "WezTerm" || s == "iTerm.app")
                || std::env::var_os("KONSOLE_VERSION").is_some())
        {
            graphics.protocol = Some("iterm2");
        }
        (graphics, keys)
    }
    pub fn clear(&self) {
        if self.protocol == Some("kitty") {
            let mut output = std::io::stdout();
            let _ = write!(output, "\x1b_Ga=d,d=I,i={},q=2;\x1b\\", self.image_id);
            let _ = output.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn geometry_is_required_and_bounded() {
        let mut g = Graphics::default();
        g.response(Response::CellSize(Some((0, 20))));
        assert_eq!(g.cell, (0, 0));
        g.response(Response::CellSize(Some((10, 20))));
        g.response(Response::Kitty);
        assert_eq!(g.cell, (10, 20));
        assert_eq!(g.protocol, Some("kitty"));
        g.response(Response::CellSize(Some((1000, 2000))));
        assert_eq!(g.cell, (10, 20));
    }
}
