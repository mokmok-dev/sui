use crate::slash::{MAX_CANDIDATES, SlashCandidate, SlashCommand};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::Text,
    widgets::{Block, Borders, Paragraph},
};
use sui_widget::PromptWidget;

/// Holds the full application state: prompt input, submitted messages, and the
/// run-loop flag.
///
/// # Example
///
/// ```no_run
/// use sui_app::App;
///
/// let mut app = App::new();
/// // Optionally customise the prompt:
/// let mut app = App::new().with_prompt_prefix("> ");
/// ```
pub struct App {
    pub(crate) input: String,
    /// Char-based cursor position within `input`.
    pub(crate) cursor_position: usize,
    pub(crate) should_quit: bool,
    pub(crate) prompt_prefix: String,
    /// History of submitted prompt texts shown in the main content area.
    pub(crate) messages: Vec<String>,
    /// Registered pluggable slash commands.
    pub(crate) plugins: Vec<Box<dyn SlashCommand>>,
    /// Candidates that match the current slash partial input.
    pub(crate) slash_candidates: Vec<SlashCandidate>,
    /// Currently highlighted index within `slash_candidates`.
    pub(crate) slash_selected: usize,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    /// Creates a new `App` with the default prompt prefix (`"❯ "`).
    #[must_use]
    pub fn new() -> Self {
        Self {
            input: String::new(),
            cursor_position: 0,
            should_quit: false,
            prompt_prefix: "❯ ".to_string(),
            messages: Vec::new(),
            plugins: Vec::new(),
            slash_candidates: Vec::new(),
            slash_selected: 0,
        }
    }

    /// Request application shutdown.
    pub const fn quit(&mut self) {
        self.should_quit = true;
    }

    /// Append a message to the message history.
    pub fn add_message(
        &mut self,
        msg: impl Into<String>,
    ) {
        self.messages.push(msg.into());
    }

    /// Register a pluggable slash command.
    ///
    /// Built-in `/exit` and `/quit` are always available. Commands registered
    /// here appear alongside them in the suggestion panel.
    pub fn register_command(
        &mut self,
        cmd: impl SlashCommand + 'static,
    ) {
        self.plugins.push(Box::new(cmd));
    }

    /// Sets the prompt prefix.
    ///
    /// The default is `"❯ "`.
    ///
    /// # Example
    ///
    /// ```
    /// use sui_app::App;
    /// let app = App::new().with_prompt_prefix("$ ");
    /// ```
    #[must_use]
    pub fn with_prompt_prefix(
        mut self,
        prefix: impl Into<String>,
    ) -> Self {
        self.prompt_prefix = prefix.into();
        self
    }

    /// Blocking run loop: draw → read event → update state → repeat until quit.
    ///
    /// # Errors
    /// Returns an I/O error if terminal operations or event reading fail.
    pub fn run(
        &mut self,
        terminal: &mut DefaultTerminal,
    ) -> std::io::Result<()> {
        while !self.should_quit {
            terminal.draw(|frame| self.render(frame))?;
            self.handle_event(&crossterm::event::read()?);
        }
        Ok(())
    }

    fn render(
        &self,
        frame: &mut Frame,
    ) {
        let area = frame.area();

        let suggestions_height = if self.slash_candidates.is_empty() {
            0
        } else {
            u16::try_from(self.slash_candidates.len().min(MAX_CANDIDATES)).unwrap_or(u16::MAX)
        };

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(3),
                Constraint::Length(suggestions_height),
            ])
            .split(area);

        // ---- main content area ----
        let main_block = Block::default()
            .borders(Borders::ALL)
            .title(" sui ")
            .style(Style::default().fg(Color::Green));

        let content: Text<'_> = if self.messages.is_empty() {
            Text::from("sui — 粋・推・遂\nType your prompt and press Enter to submit.")
        } else {
            let joined: String = self
                .messages
                .iter()
                .map(|m| format!("❯ {m}"))
                .collect::<Vec<_>>()
                .join("\n");
            Text::from(joined)
        };

        let main_paragraph = Paragraph::new(content).block(main_block);
        frame.render_widget(main_paragraph, layout[0]);

        // ---- prompt area ----
        let prompt = PromptWidget::new(&self.input, self.cursor_position, &self.prompt_prefix);
        let cursor_pos = prompt.screen_cursor(layout[1]);
        frame.render_widget(prompt, layout[1]);
        frame.set_cursor_position((cursor_pos.0, cursor_pos.1));

        // ---- slash-suggestions area ----
        if !self.slash_candidates.is_empty() {
            self.render_slash_suggestions(frame, layout[2]);
        }
    }
}
