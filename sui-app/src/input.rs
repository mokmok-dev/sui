use crate::{App, char_index_to_byte};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

impl App {
    pub fn handle_event(
        &mut self,
        event: &Event,
    ) {
        if let Event::Key(key) = event {
            self.handle_key(*key);
        }
    }

    pub(crate) fn handle_key(
        &mut self,
        key: KeyEvent,
    ) {
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
            },
            KeyCode::Esc => self.should_quit = true,
            KeyCode::Enter => {
                if !self.input.is_empty() {
                    self.messages.push(self.input.clone());
                    self.input.clear();
                    self.cursor_position = 0;
                }
            },
            KeyCode::Backspace => {
                if self.cursor_position > 0 {
                    let new_pos = self.cursor_position.saturating_sub(1);
                    if let Some(byte_idx) = char_index_to_byte(&self.input, new_pos) {
                        self.input.remove(byte_idx);
                        self.cursor_position = new_pos;
                    }
                }
            },
            KeyCode::Delete => {
                if self.cursor_position < self.input.chars().count()
                    && let Some(byte_idx) = char_index_to_byte(&self.input, self.cursor_position)
                {
                    self.input.remove(byte_idx);
                }
            },
            KeyCode::Left => {
                if self.cursor_position > 0 {
                    self.cursor_position = self.cursor_position.saturating_sub(1);
                }
            },
            KeyCode::Right => {
                if self.cursor_position < self.input.chars().count() {
                    self.cursor_position = self.cursor_position.saturating_add(1);
                }
            },
            KeyCode::Home => self.cursor_position = 0,
            KeyCode::End => self.cursor_position = self.input.chars().count(),
            KeyCode::Char(c) => {
                let byte_pos = char_index_to_byte(&self.input, self.cursor_position)
                    .unwrap_or(self.input.len());
                self.input.insert(byte_pos, c);
                self.cursor_position = self.cursor_position.saturating_add(1);
            },
            _ => {},
        }
    }
}
