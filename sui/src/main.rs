use sui_app::{App, PROMPT_HEIGHT};
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum MainError {
    #[error(transparent)]
    Eyre(#[from] color_eyre::eyre::Report),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[tokio::main]
async fn main() -> Result<(), MainError> {
    color_eyre::install().map_err(MainError::from)?;

    let mut app = App::new();
    // Inline viewport on the normal screen (no alternate buffer / fullscreen).
    // Starts as the prompt only; App grows it when slash suggestions appear.
    // Submitted lines are inserted above the viewport and scroll into scrollback,
    // pinning the prompt at the bottom once the screen fills — Codex-style.
    let mut terminal = ratatui::try_init_with_options(ratatui::TerminalOptions {
        viewport: ratatui::Viewport::Inline(PROMPT_HEIGHT),
    })?;

    let result = app.run(&mut terminal);
    ratatui::restore();
    result.map_err(MainError::from)
}
