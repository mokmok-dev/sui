use crate::App;
use ratatui::{Frame, layout::Rect, style::Style, widgets::Paragraph};
use sui_theme::Theme;

pub const MAX_CANDIDATES: usize = 5;

/// Built-in slash commands: `(name, description)`.
///
/// `/exit` and `/quit` both shut down the application.
const BUILTIN_COMMANDS: &[(&str, &str)] = &[
    ("exit", "quit the application"),
    ("quit", "quit the application"),
];

/// A pluggable slash command.
///
/// Implement this trait and register with [`App::register_command`] to add
/// custom slash commands.
///
/// # Example
///
/// ```
/// use sui_app::{App, SlashCommand};
///
/// struct Hello;
///
/// impl SlashCommand for Hello {
///     fn name(&self) -> &'static str { "hello" }
///     fn description(&self) -> &'static str { "print a greeting" }
///     fn execute(&self, app: &mut App) {
///         app.add_message("hello, world!");
///     }
/// }
///
/// let mut app = App::new();
/// app.register_command(Hello);
/// ```
pub trait SlashCommand {
    /// The command name as typed after `/` (e.g. `"skill"` for `/skill`).
    fn name(&self) -> &'static str;

    /// A short description shown in the suggestion panel.
    fn description(&self) -> &'static str;

    /// Called when the user selects and executes this command.
    fn execute(
        &self,
        app: &mut App,
    );
}

/// Internal representation of a single suggestion entry.
#[derive(Clone, Copy, Debug)]
pub(crate) enum SlashCandidate {
    /// Index into [`BUILTIN_COMMANDS`].
    Builtin { index: usize },
    /// Index into [`App::plugins`].
    Plugin { index: usize },
}

/// No-op command used as a placeholder during plugin execution.
pub(crate) struct NoopCommand;

impl SlashCommand for NoopCommand {
    fn name(&self) -> &'static str {
        ""
    }

    fn description(&self) -> &'static str {
        ""
    }

    fn execute(
        &self,
        _app: &mut App,
    ) {
    }
}

impl App {
    /// Renders the slash-command suggestion lines directly under the prompt.
    pub(crate) fn render_slash_suggestions(
        &self,
        frame: &mut Frame,
        area: Rect,
    ) {
        let selected_style = Theme::DEFAULT.selected_style();
        let normal_style = Style::default();

        for (i, candidate) in self.slash_candidates.iter().enumerate() {
            let (name, desc) = match candidate {
                SlashCandidate::Builtin { index } => BUILTIN_COMMANDS[*index],
                SlashCandidate::Plugin { index } => {
                    let cmd = &self.plugins[*index];
                    (cmd.name(), cmd.description())
                },
            };
            let text = format!(" /{name} — {desc}");
            let style = if i == self.slash_selected {
                selected_style
            } else {
                normal_style
            };
            let line_area = Rect::new(
                area.x,
                area.y + u16::try_from(i).unwrap_or(u16::MAX),
                area.width,
                1,
            );
            frame.render_widget(Paragraph::new(text).style(style), line_area);
        }
    }

    /// Rebuilds `slash_candidates` based on the current input.
    ///
    /// Slash suggestions only apply in [`crate::Mode::Prompt`].
    pub(crate) fn update_slash_candidates(&mut self) {
        if self.mode != crate::Mode::Prompt {
            self.slash_candidates.clear();
            self.slash_selected = 0;
            return;
        }
        if let Some(partial) = self.input.strip_prefix('/') {
            self.slash_candidates.clear();

            // Built-ins are always checked first.
            for (i, (name, _)) in BUILTIN_COMMANDS.iter().enumerate() {
                if self.slash_candidates.len() >= MAX_CANDIDATES {
                    break;
                }
                if name.starts_with(partial) {
                    self.slash_candidates
                        .push(SlashCandidate::Builtin { index: i });
                }
            }

            // Plugin commands.
            for (i, cmd) in self.plugins.iter().enumerate() {
                if self.slash_candidates.len() >= MAX_CANDIDATES {
                    break;
                }
                if cmd.name().starts_with(partial) {
                    self.slash_candidates
                        .push(SlashCandidate::Plugin { index: i });
                }
            }

            let len = self.slash_candidates.len().max(1);
            if self.slash_selected >= len {
                self.slash_selected = 0;
            }
        } else {
            self.slash_candidates.clear();
            self.slash_selected = 0;
        }
    }

    /// Executes the currently highlighted slash candidate.
    pub(crate) fn execute_selected_slash_command(&mut self) {
        // Extract the action outside the borrow scope.
        let action = {
            let candidate = &self.slash_candidates[self.slash_selected];
            match candidate {
                SlashCandidate::Builtin { .. } => None,
                SlashCandidate::Plugin { index } => Some(*index),
            }
        };
        match action {
            None => self.quit(),
            Some(index) => {
                // Temporarily swap out the command so we can call
                // execute(&mut self) without conflicting borrows.
                let cmd = std::mem::replace(&mut self.plugins[index], Box::new(NoopCommand));
                cmd.execute(self);
                self.plugins[index] = cmd;
            },
        }
    }

    /// Returns the command name of the currently selected slash candidate.
    pub(crate) fn selected_candidate_name(&self) -> &'static str {
        match &self.slash_candidates[self.slash_selected] {
            SlashCandidate::Builtin { index } => BUILTIN_COMMANDS[*index].0,
            SlashCandidate::Plugin { index } => self.plugins[*index].name(),
        }
    }

    pub(crate) fn handle_slash_command(
        &mut self,
        cmd: &str,
    ) {
        if BUILTIN_COMMANDS.iter().any(|(name, _)| *name == cmd) {
            self.quit();
        } else {
            self.add_message(format!("unknown command: /{cmd}"));
        }
    }
}
