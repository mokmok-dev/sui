use crate::App;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    widgets::Paragraph,
};

pub const MAX_CANDIDATES: usize = 5;

/// Internal representation of a single suggestion entry.
#[derive(Clone, Debug)]
pub(crate) enum SlashCandidate {
    /// The built-in `/exit` command.
    Builtin,
}

impl App {
    /// Renders the slash-command suggestion lines directly under the prompt.
    pub(crate) fn render_slash_suggestions(
        &self,
        frame: &mut Frame,
        area: Rect,
    ) {
        let selected_style = Style::default().fg(Color::Black).bg(Color::Yellow);
        let normal_style = Style::default();

        for (i, candidate) in self.slash_candidates.iter().enumerate() {
            let (name, desc) = match candidate {
                SlashCandidate::Builtin => ("exit", "quit the application"),
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
    pub(crate) fn update_slash_candidates(&mut self) {
        if let Some(partial) = self.input.strip_prefix('/') {
            self.slash_candidates.clear();

            if "exit".starts_with(partial) {
                self.slash_candidates.push(SlashCandidate::Builtin);
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
        match self.slash_candidates[self.slash_selected] {
            SlashCandidate::Builtin => self.quit(),
        }
    }

    /// Returns the command name of the currently selected slash candidate.
    pub(crate) fn selected_candidate_name(&self) -> &'static str {
        match &self.slash_candidates[self.slash_selected] {
            SlashCandidate::Builtin => "exit",
        }
    }

    pub(crate) fn handle_slash_command(
        &mut self,
        cmd: &str,
    ) {
        match cmd {
            "exit" => self.quit(),
            unknown => {
                self.add_message(format!("unknown command: /{unknown}"));
            },
        }
    }
}
