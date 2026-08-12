use crate::mode::Mode;
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

    pub(crate) fn handle_enter(&mut self) {
        match self.mode {
            Mode::Shell => {
                let command = self.input.clone();
                self.handle_shell_command(&command);
            },
            Mode::Prompt => {
                if self.input.is_empty() {
                    return;
                }
                if !self.slash_candidates.is_empty() {
                    self.execute_selected_slash_command();
                } else if self.input.starts_with('/') {
                    let cmd = self.input[1..].to_owned();
                    self.handle_slash_command(&cmd);
                } else {
                    let prompt = self.input.clone();
                    self.handle_chat_prompt(&prompt);
                }
            },
        }
        self.input.clear();
        self.cursor_position = 0;
        self.slash_candidates.clear();
        self.slash_selected = 0;
    }

    /// Sends a user turn to the configured LLM and appends the reply as
    /// [`crate::app::ScrollbackLine::Reply`] lines.
    ///
    /// Blocks the event loop until the Proxy responds or hits
    /// [`crate::llm::DEFAULT_CHAT_TIMEOUT`]. Streaming is intentionally out of
    /// scope for this sync TUI path.
    pub(crate) fn handle_chat_prompt(
        &mut self,
        prompt: &str,
    ) {
        self.add_message(prompt);
        let Some(client) = self.llm.clone() else {
            self.add_message(
                "llm not configured: set LITELLM_BASE_URL and LITELLM_MODEL (optional LITELLM_API_KEY)",
            );
            return;
        };

        self.chat_history
            .push(sui_llm::ChatMessage::user(prompt.to_owned()));
        match crate::llm::chat_blocking(&client, &self.chat_history) {
            Ok(response) => {
                self.chat_history
                    .push(sui_llm::ChatMessage::assistant(response.content.clone()));
                for line in response.content.lines() {
                    self.add_reply(line);
                }
            },
            Err(error) => {
                let _ = self.chat_history.pop();
                self.add_message(format!("llm error: {error}"));
            },
        }
    }

    /// Runs a one-shot shell command via [`crate::bang`].
    ///
    /// Blocks the event loop until the command finishes or hits the default
    /// timeout ([`sui_tools::DEFAULT_RUN_TIMEOUT`]). Long-running commands will
    /// freeze the TUI until they exit or are killed by that deadline.
    pub(crate) fn handle_shell_command(
        &mut self,
        command: &str,
    ) {
        let command = command.trim();
        if command.is_empty() {
            self.add_message("usage: <command>");
            return;
        }
        self.add_message(format!("! {command}"));
        match crate::bang::run_blocking(command) {
            Ok(output) => {
                for line in crate::bang::format_output(&output) {
                    self.add_ghost(line);
                }
            },
            Err(error) => self.add_message(format!("bash error: {error}")),
        }
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) fn handle_key(
        &mut self,
        key: KeyEvent,
    ) {
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
            },
            KeyCode::Esc => {
                if matches!(self.mode, Mode::Prompt) {
                    self.should_quit = true;
                } else {
                    // Shell (and future modes): leave back to Prompt.
                    self.set_mode(Mode::Prompt);
                }
            },
            KeyCode::Enter => self.handle_enter(),
            KeyCode::Backspace => {
                if self.cursor_position > 0 {
                    let new_pos = self.cursor_position.saturating_sub(1);
                    if let Some(byte_idx) = char_index_to_byte(&self.input, new_pos) {
                        self.input.remove(byte_idx);
                        self.cursor_position = new_pos;
                    }
                }
                self.update_slash_candidates();
            },
            KeyCode::Delete => {
                if self.cursor_position < self.input.chars().count()
                    && let Some(byte_idx) = char_index_to_byte(&self.input, self.cursor_position)
                {
                    self.input.remove(byte_idx);
                }
                self.update_slash_candidates();
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
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cursor_position = 0;
            },
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cursor_position = self.input.chars().count();
            },
            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.cursor_position < self.input.chars().count() {
                    self.cursor_position = self.cursor_position.saturating_add(1);
                }
            },
            KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.cursor_position > 0 {
                    self.cursor_position = self.cursor_position.saturating_sub(1);
                }
            },
            KeyCode::Char('h') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.cursor_position > 0 {
                    let new_pos = self.cursor_position.saturating_sub(1);
                    if let Some(byte_idx) = char_index_to_byte(&self.input, new_pos) {
                        self.input.remove(byte_idx);
                        self.cursor_position = new_pos;
                    }
                }
                self.update_slash_candidates();
            },
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.cursor_position < self.input.chars().count()
                    && let Some(byte_idx) = char_index_to_byte(&self.input, self.cursor_position)
                {
                    self.input.remove(byte_idx);
                }
                self.update_slash_candidates();
            },
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.cursor_position < self.input.chars().count() {
                    let start_byte = char_index_to_byte(&self.input, self.cursor_position)
                        .unwrap_or(self.input.len());
                    self.input.truncate(start_byte);
                }
                self.update_slash_candidates();
            },
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if !self.slash_candidates.is_empty() {
                    let len = self.slash_candidates.len();
                    self.slash_selected = (self.slash_selected + 1) % len;
                }
            },
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if !self.slash_candidates.is_empty() {
                    let len = self.slash_candidates.len();
                    self.slash_selected = (self.slash_selected + len - 1) % len;
                }
            },
            // Empty prompt + `!` enters shell mode (sticky), like vim `:` for cmdline.
            KeyCode::Char('!')
                if self.mode == Mode::Prompt
                    && self.input.is_empty()
                    && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.set_mode(Mode::Shell);
            },
            KeyCode::Char(c) => {
                let byte_pos = char_index_to_byte(&self.input, self.cursor_position)
                    .unwrap_or(self.input.len());
                self.input.insert(byte_pos, c);
                self.cursor_position = self.cursor_position.saturating_add(1);
                self.update_slash_candidates();
            },
            KeyCode::Tab if !self.slash_candidates.is_empty() => {
                let name = self.selected_candidate_name();
                self.input = format!("/{name}");
                self.cursor_position = self.input.chars().count();
                self.slash_selected = (self.slash_selected + 1) % self.slash_candidates.len();
                self.update_slash_candidates();
            },
            KeyCode::Down if !self.slash_candidates.is_empty() => {
                let len = self.slash_candidates.len();
                self.slash_selected = (self.slash_selected + 1) % len;
            },
            KeyCode::BackTab | KeyCode::Up if !self.slash_candidates.is_empty() => {
                let len = self.slash_candidates.len();
                self.slash_selected = (self.slash_selected + len - 1) % len;
            },
            _ => {},
        }
    }
}
