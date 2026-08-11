use crate::slash::{MAX_CANDIDATES, SlashCandidate, SlashCommand};
use ratatui::{
    DefaultTerminal, Frame, Terminal, TerminalOptions, Viewport,
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Layout, Position},
    widgets::{Paragraph, Widget},
};
use sui_widget::PromptWidget;

/// Rows occupied by the bordered prompt widget.
pub const PROMPT_HEIGHT: u16 = 3;

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
    /// History of submitted prompts / status lines.
    ///
    /// New entries are committed above the inline viewport via
    /// [`Terminal::insert_before`] so they scroll into the terminal scrollback
    /// while the prompt stays pinned once it reaches the bottom of the screen.
    pub(crate) messages: Vec<String>,
    /// How many messages have already been written above the viewport.
    pub(crate) flushed_messages: usize,
    /// Current [`Viewport::Inline`] height managed by [`App::run`].
    pub(crate) viewport_height: u16,
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
            flushed_messages: 0,
            viewport_height: PROMPT_HEIGHT,
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
    ///
    /// The next [`App::run`] iteration (or [`App::flush_messages`]) writes it
    /// above the inline prompt into the terminal scrollback.
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

    /// Inline viewport height for the current UI: prompt plus slash suggestions.
    #[must_use]
    pub fn inline_height(&self) -> u16 {
        let suggestions =
            u16::try_from(self.slash_candidates.len().min(MAX_CANDIDATES)).unwrap_or(u16::MAX);
        PROMPT_HEIGHT.saturating_add(suggestions)
    }

    /// Blocking run loop: flush scrollback → sync viewport → draw → read event.
    ///
    /// Expects a terminal initialized with [`PROMPT_HEIGHT`] via
    /// [`ratatui::init_with_options`] and [`Viewport::Inline`] so the UI stays
    /// in the normal screen buffer (Codex-style), not the alternate screen.
    ///
    /// # Errors
    /// Returns an I/O error if terminal operations or event reading fail.
    pub fn run(
        &mut self,
        terminal: &mut DefaultTerminal,
    ) -> std::io::Result<()> {
        while !self.should_quit {
            self.flush_messages(terminal)?;
            self.sync_viewport_height(terminal)?;
            terminal.draw(|frame| self.render(frame))?;
            self.handle_event(&crossterm::event::read()?);
        }
        Ok(())
    }

    /// Writes any unflushed messages above the inline viewport.
    ///
    /// Once the viewport reaches the bottom of the terminal, further inserts
    /// scroll prior output into the scrollback buffer and keep the prompt pinned.
    ///
    /// # Errors
    /// Returns a backend error if inserting lines fails.
    pub fn flush_messages<B: Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> Result<(), B::Error> {
        while self.flushed_messages < self.messages.len() {
            let line = format!(
                "{}{}",
                self.prompt_prefix, self.messages[self.flushed_messages]
            );
            terminal.insert_before(1, move |buf| {
                Paragraph::new(line).render(buf.area, buf);
            })?;
            self.flushed_messages += 1;
        }
        Ok(())
    }

    /// Grows or shrinks the inline viewport to match [`App::inline_height`].
    ///
    /// Ratatui fixes [`Viewport::Inline`] height at construction time, so a
    /// height change rebuilds the terminal on the normal screen (raw mode stays
    /// enabled; only the viewport is replaced).
    fn sync_viewport_height(
        &mut self,
        terminal: &mut DefaultTerminal,
    ) -> std::io::Result<()> {
        let height = self.inline_height();
        if height == self.viewport_height {
            return Ok(());
        }

        let area = terminal.get_frame().area();
        terminal.clear()?;
        terminal.set_cursor_position(Position::new(area.x, area.y))?;

        let backend = CrosstermBackend::new(std::io::stdout());
        *terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Inline(height),
            },
        )?;
        self.viewport_height = height;
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

        let [prompt_area, suggestions_area] = Layout::vertical([
            Constraint::Length(PROMPT_HEIGHT),
            Constraint::Length(suggestions_height),
        ])
        .areas(area);

        let prompt = PromptWidget::new(&self.input, self.cursor_position, &self.prompt_prefix);
        let cursor_pos = prompt.screen_cursor(prompt_area);
        frame.render_widget(prompt, prompt_area);
        frame.set_cursor_position((cursor_pos.0, cursor_pos.1));

        if !self.slash_candidates.is_empty() {
            self.render_slash_suggestions(frame, suggestions_area);
        }
    }
}
