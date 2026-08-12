use crate::mode::Mode;
use crate::slash::{MAX_CANDIDATES, SlashCandidate, SlashCommand};
use ratatui::{
    DefaultTerminal, Frame, Terminal, TerminalOptions, Viewport,
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Layout, Position},
    style::{Modifier, Style},
    widgets::{Paragraph, Widget},
};
use sui_llm::{ChatMessage, LlmClient};
use sui_widget::PromptWidget;

/// Rows occupied by the bordered prompt widget.
pub const PROMPT_HEIGHT: u16 = 3;

/// Extra inline rows reserved while the slash-suggestion panel is open.
///
/// Kept as a `u16` literal (not `MAX_CANDIDATES as u16`) to satisfy pedantic
/// cast lints; the assert below locks it to [`MAX_CANDIDATES`].
const SUGGESTION_PANEL_HEIGHT: u16 = 5;
const _: () = assert!(SUGGESTION_PANEL_HEIGHT as usize == MAX_CANDIDATES);

/// A single scrollback line pending flush above the inline viewport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ScrollbackLine {
    /// User / status line rendered with the prompt prefix.
    Prompt(String),
    /// Dim ghost text (shell stdout/stderr) without the prompt prefix.
    Ghost(String),
    /// Assistant reply text without the prompt prefix (normal intensity).
    Reply(String),
}

/// Holds the full application state: prompt input, message history, and the
/// run-loop flag.
///
/// # Example
///
/// ```no_run
/// use sui_app::App;
/// use sui_llm::LlmClient;
///
/// let mut app = App::new();
/// if let Ok(client) = LlmClient::from_env() {
///     app = app.with_llm(client);
/// }
/// app.run_inline()?;
/// # Ok::<(), std::io::Error>(())
/// ```
pub struct App {
    pub(crate) input: String,
    /// Char-based cursor position within `input`.
    pub(crate) cursor_position: usize,
    pub(crate) should_quit: bool,
    pub(crate) prompt_prefix: String,
    /// Sticky interaction mode (see [`Mode`]).
    pub(crate) mode: Mode,
    /// History of submitted prompts / status / ghost lines.
    ///
    /// New entries are committed above the inline viewport via
    /// [`Terminal::insert_before`] so they scroll into the terminal scrollback
    /// while the prompt stays pinned once it reaches the bottom of the screen.
    pub(crate) messages: Vec<ScrollbackLine>,
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
    /// Optional `LiteLLM` Proxy client for [`Mode::Prompt`] chat.
    pub(crate) llm: Option<LlmClient>,
    /// Session chat turns sent to the Proxy (user + assistant only).
    pub(crate) chat_history: Vec<ChatMessage>,
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
            mode: Mode::Prompt,
            messages: Vec::new(),
            flushed_messages: 0,
            viewport_height: PROMPT_HEIGHT,
            plugins: Vec::new(),
            slash_candidates: Vec::new(),
            slash_selected: 0,
            llm: None,
            chat_history: Vec::new(),
        }
    }

    /// Attach an LLM client for prompt-mode chat.
    ///
    /// Without this, prompt submits surface a configuration hint instead of
    /// calling the Proxy. Typical wiring:
    /// `App::new().with_llm(LlmClient::from_env()?)`.
    #[must_use]
    pub fn with_llm(
        mut self,
        client: LlmClient,
    ) -> Self {
        self.llm = Some(client);
        self
    }

    /// Request application shutdown.
    pub const fn quit(&mut self) {
        self.should_quit = true;
    }

    /// Current sticky interaction mode.
    #[must_use]
    pub const fn mode(&self) -> Mode {
        self.mode
    }

    /// Switch mode, clearing the input buffer and slash suggestions.
    pub(crate) fn set_mode(
        &mut self,
        mode: Mode,
    ) {
        self.mode = mode;
        self.input.clear();
        self.cursor_position = 0;
        self.slash_candidates.clear();
        self.slash_selected = 0;
    }

    /// Append a normal (prompt-prefixed) message to the scrollback history.
    ///
    /// The next [`App::run`] iteration (or [`App::flush_messages`]) writes it
    /// above the inline prompt into the terminal scrollback.
    pub fn add_message(
        &mut self,
        msg: impl Into<String>,
    ) {
        self.messages.push(ScrollbackLine::Prompt(msg.into()));
    }

    /// Append dim ghost text (e.g. shell command output) to the scrollback.
    pub fn add_ghost(
        &mut self,
        msg: impl Into<String>,
    ) {
        self.messages.push(ScrollbackLine::Ghost(msg.into()));
    }

    /// Append an assistant reply line (no prompt prefix, normal intensity).
    pub fn add_reply(
        &mut self,
        msg: impl Into<String>,
    ) {
        self.messages.push(ScrollbackLine::Reply(msg.into()));
    }

    /// Border title for the current mode.
    #[must_use]
    pub(crate) const fn prompt_title(&self) -> &'static str {
        self.mode.title()
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

    /// Inline viewport height for the current UI.
    ///
    /// Prompt-only while idle; expands by a fixed suggestion-panel budget when
    /// any slash candidates are visible (avoids resizing on every keystroke).
    #[must_use]
    pub const fn inline_height(&self) -> u16 {
        if self.slash_candidates.is_empty() {
            PROMPT_HEIGHT
        } else {
            PROMPT_HEIGHT.saturating_add(SUGGESTION_PANEL_HEIGHT)
        }
    }

    /// Initialize an inline terminal, run until quit, then restore the terminal.
    ///
    /// This is the preferred entry point: no alternate screen, prompt-only
    /// viewport, scrollback via [`App::flush_messages`].
    ///
    /// On exit only raw mode is disabled. [`ratatui::restore`] is intentionally
    /// not used: it always emits `LeaveAlternateScreen` (`CSI ?1049l`), and many
    /// terminals treat that as “restore cursor” even when the app never entered
    /// the alternate buffer — yanking the cursor above `insert_before`
    /// scrollback (including shell ghost lines).
    ///
    /// # Errors
    /// Returns an I/O error if terminal setup or the run loop fails. Raw-mode
    /// cleanup is best-effort and does not override a successful run result.
    pub fn run_inline(&mut self) -> std::io::Result<()> {
        let mut terminal = ratatui::try_init_with_options(TerminalOptions {
            viewport: Viewport::Inline(PROMPT_HEIGHT),
        })?;
        let result = self.run(&mut terminal);
        let _ = crossterm::terminal::disable_raw_mode();
        result
    }

    /// Blocking run loop: flush scrollback → sync viewport → draw → read event.
    ///
    /// Prefer [`App::run_inline`] unless you already own an inline
    /// [`ratatui::Viewport`] terminal whose height starts at [`PROMPT_HEIGHT`].
    ///
    /// On exit any pending scrollback (including ghost lines) is flushed, the
    /// inline viewport is cleared, and the cursor is moved to the viewport
    /// origin so the host shell prompt sits just below that output.
    ///
    /// Callers that set up the terminal themselves should disable raw mode
    /// after this returns and **must not** call [`ratatui::restore`] (it emits
    /// `LeaveAlternateScreen`, which can reset the cursor above the scrollback).
    ///
    /// # Errors
    /// Returns an I/O error if terminal operations or event reading fail.
    pub fn run(
        &mut self,
        terminal: &mut DefaultTerminal,
    ) -> std::io::Result<()> {
        let result = self.event_loop(terminal);
        // Commit any lines queued by the final event (e.g. bang ghosts) before
        // parking the cursor — otherwise teardown uses a stale viewport origin.
        let _ = self.flush_messages(terminal);
        let _ = Self::teardown_inline(terminal);
        result
    }

    fn event_loop(
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

    /// Clears the inline viewport and parks the cursor at its top-left origin.
    ///
    /// [`Terminal::clear`] restores the previous cursor position afterward, so
    /// callers must move back to the viewport origin explicitly (same pattern
    /// as Atuin’s inline-search teardown).
    pub(crate) fn teardown_inline<B: Backend>(terminal: &mut Terminal<B>) -> Result<(), B::Error> {
        let origin = terminal.get_frame().area().as_position();
        terminal.clear()?;
        terminal.set_cursor_position(origin)?;
        Ok(())
    }

    /// Writes any unflushed messages above the inline viewport.
    ///
    /// Prompt lines include [`Self::prompt_prefix`]. Ghost lines are dim and
    /// unprefixed.
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
            let line = &self.messages[self.flushed_messages];
            match line {
                ScrollbackLine::Prompt(text) => {
                    let rendered = format!("{}{text}", self.prompt_prefix);
                    terminal.insert_before(1, move |buf| {
                        Paragraph::new(rendered).render(buf.area, buf);
                    })?;
                },
                ScrollbackLine::Ghost(text) => {
                    let rendered = text.clone();
                    terminal.insert_before(1, move |buf| {
                        Paragraph::new(rendered)
                            .style(Style::default().add_modifier(Modifier::DIM))
                            .render(buf.area, buf);
                    })?;
                },
                ScrollbackLine::Reply(text) => {
                    let rendered = text.clone();
                    terminal.insert_before(1, move |buf| {
                        Paragraph::new(rendered).render(buf.area, buf);
                    })?;
                },
            }
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

        let [prompt_area, suggestions_area, _reserved] = Layout::vertical([
            Constraint::Length(PROMPT_HEIGHT),
            Constraint::Length(suggestions_height),
            Constraint::Min(0),
        ])
        .areas(area);

        let prompt = PromptWidget::new(&self.input, self.cursor_position, &self.prompt_prefix)
            .with_title(self.prompt_title());
        let cursor_pos = prompt.screen_cursor(prompt_area);
        frame.render_widget(prompt, prompt_area);
        frame.set_cursor_position((cursor_pos.0, cursor_pos.1));

        if !self.slash_candidates.is_empty() {
            self.render_slash_suggestions(frame, suggestions_area);
        }
    }
}
