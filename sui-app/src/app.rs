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
        }
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

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(3)])
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
    }
}
