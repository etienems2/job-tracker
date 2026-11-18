//! Last snapshot plus the list, command line, and overlay state.

use std::path::PathBuf;

use ratatui::widgets::TableState;
use unicode_segmentation::UnicodeSegmentation;

use crate::domain::{Application, Filter, Snapshot, Sort, cmp_applications};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Normal,
    Command,
    StagePick,
    Help,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandLine {
    pub(crate) buffer: String,
    /// Byte index on a grapheme boundary.
    pub(crate) cursor: usize,
}

impl CommandLine {
    pub(crate) fn new() -> Self {
        Self {
            buffer: String::new(),
            cursor: 0,
        }
    }

    pub(crate) fn set(&mut self, text: impl Into<String>) {
        self.buffer = text.into();
        self.cursor = self.buffer.len();
    }

    pub(crate) fn clear(&mut self) {
        self.buffer.clear();
        self.cursor = 0;
    }

    pub(crate) fn insert(&mut self, text: &str) {
        self.buffer.insert_str(self.cursor, text);
        self.cursor += text.len();
    }

    pub(crate) fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let previous = prev_boundary(&self.buffer, self.cursor);
        self.buffer.replace_range(previous..self.cursor, "");
        self.cursor = previous;
    }

    pub(crate) fn delete_forward(&mut self) {
        if self.cursor >= self.buffer.len() {
            return;
        }
        let next = next_boundary(&self.buffer, self.cursor);
        self.buffer.replace_range(self.cursor..next, "");
    }

    pub(crate) fn move_left(&mut self) {
        self.cursor = prev_boundary(&self.buffer, self.cursor);
    }

    pub(crate) fn move_right(&mut self) {
        self.cursor = next_boundary(&self.buffer, self.cursor);
    }

    pub(crate) fn move_home(&mut self) {
        self.cursor = 0;
    }

    pub(crate) fn move_end(&mut self) {
        self.cursor = self.buffer.len();
    }
}

fn boundaries(text: &str) -> Vec<usize> {
    let mut bounds = vec![0];
    let mut end = 0;
    for grapheme in text.graphemes(true) {
        end += grapheme.len();
        bounds.push(end);
    }
    bounds
}

fn prev_boundary(text: &str, cursor: usize) -> usize {
    boundaries(text)
        .into_iter()
        .take_while(|bound| *bound < cursor)
        .last()
        .unwrap_or(0)
}

fn next_boundary(text: &str, cursor: usize) -> usize {
    boundaries(text)
        .into_iter()
        .find(|bound| *bound > cursor)
        .unwrap_or(text.len())
}

#[derive(Debug)]
pub(crate) struct Model {
    pub(crate) apps: Vec<Application>,
    pub(crate) selection: usize,
    pub(crate) table_state: TableState,
    pub(crate) mode: Mode,
    pub(crate) command: CommandLine,
    pub(crate) filter: Filter,
    pub(crate) sort: Sort,
    pub(crate) status: String,
    pub(crate) status_error: bool,
    pub(crate) running: bool,
    pub(crate) show_detail: bool,
    pub(crate) db_path: PathBuf,
}

impl Model {
    pub(crate) fn new(snapshot: Snapshot, db_path: PathBuf) -> Self {
        let mut model = Self {
            apps: snapshot.applications,
            selection: 0,
            table_state: TableState::default(),
            mode: Mode::Normal,
            command: CommandLine::new(),
            filter: Filter::Active,
            sort: Sort::Recent,
            status: db_path.display().to_string(),
            status_error: false,
            running: true,
            show_detail: false,
            db_path,
        };
        model.sync_table();
        model
    }

    pub(crate) fn visible_indices(&self) -> Vec<usize> {
        let mut indices: Vec<usize> = self
            .apps
            .iter()
            .enumerate()
            .filter(|(_, app)| app.matches(&self.filter))
            .map(|(index, _)| index)
            .collect();
        indices.sort_by(|&left, &right| {
            cmp_applications(&self.apps[left], &self.apps[right], self.sort)
        });
        indices
    }

    pub(crate) fn selected_id(&self) -> Option<i64> {
        let indices = self.visible_indices();
        indices
            .get(self.selection)
            .map(|&index| self.apps[index].id)
    }

    pub(crate) fn selected_app(&self) -> Option<&Application> {
        let indices = self.visible_indices();
        indices.get(self.selection).map(|&index| &self.apps[index])
    }

    pub(crate) fn select_id(&mut self, id: i64) {
        let indices = self.visible_indices();
        self.selection = indices
            .into_iter()
            .position(|index| self.apps[index].id == id)
            .unwrap_or(0);
        self.sync_table();
    }

    pub(crate) fn sync_table(&mut self) {
        let count = self.visible_indices().len();
        if count == 0 {
            self.selection = 0;
            self.table_state.select(None);
        } else {
            if self.selection >= count {
                self.selection = count - 1;
            }
            self.table_state.select(Some(self.selection));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backspace_removes_one_grapheme() {
        let mut line = CommandLine::new();
        line.set("café");
        line.backspace();
        assert_eq!(line.buffer, "caf");
        assert_eq!(line.cursor, "caf".len());
    }

    #[test]
    fn cursor_moves_by_grapheme_around_a_combining_mark() {
        let mut line = CommandLine::new();
        line.insert("e\u{0301}");
        assert_eq!(line.cursor, "e\u{0301}".len());
        line.move_left();
        assert_eq!(line.cursor, 0);
        line.insert("a");
        assert_eq!(line.buffer, "ae\u{0301}");
        // The cursor sits after "a", so forward-delete removes the whole
        // combining cluster rather than only the base letter.
        line.delete_forward();
        assert_eq!(line.buffer, "a");
    }

    #[test]
    fn moving_past_either_end_stays_put() {
        let mut line = CommandLine::new();
        line.set("ab");
        line.move_right();
        assert_eq!(line.cursor, 2);
        line.move_home();
        line.move_left();
        assert_eq!(line.cursor, 0);
        line.backspace();
        assert_eq!(line.buffer, "ab");
    }
}
