//! One wrapping index for the current document and terminal width; no text copies.
use crate::lyrics::Lyrics;
use std::{
    cell::{Ref, RefCell},
    ops::Range,
    sync::{Arc, Weak},
};
use unicode_width::UnicodeWidthChar;
#[derive(Default)]
struct Layout {
    document: Weak<Lyrics>,
    width: u16,
    rows: Vec<(usize, Range<usize>)>,
}
#[derive(Default)]
pub(super) struct WrappedLyrics(RefCell<Layout>);
impl WrappedLyrics {
    pub fn clear(&self) {
        *self.0.borrow_mut() = Layout::default();
    }
    pub fn rows(&self, document: &Arc<Lyrics>, width: u16) -> Ref<'_, Vec<(usize, Range<usize>)>> {
        let key = Arc::downgrade(document);
        {
            let mut layout = self.0.borrow_mut();
            if layout.width != width || !layout.document.ptr_eq(&key) {
                layout.rows.clear();
                for (line, lyric) in document.lines.iter().enumerate() {
                    let mut start = 0;
                    let mut columns = 0;
                    for (i, c) in lyric.text.char_indices() {
                        let n = c.width().unwrap_or(0);
                        if columns + n > usize::from(width.max(1)) && i > start {
                            layout.rows.push((line, start..i));
                            start = i;
                            columns = 0;
                        }
                        columns += n;
                    }
                    layout.rows.push((line, start..lyric.text.len()));
                }
                layout.document = key;
                layout.width = width;
            }
        }
        Ref::map(self.0.borrow(), |layout| &layout.rows)
    }
}
