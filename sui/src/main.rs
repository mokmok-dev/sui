use sui_app::App;
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
    ratatui::run(|terminal| app.run(terminal)).map_err(MainError::from)?;

    Ok(())
}
