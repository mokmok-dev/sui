use sui_app::App;
use sui_llm::LlmClient;
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
    match LlmClient::from_env() {
        Ok(client) => app = app.with_llm(client),
        Err(error) => {
            // Shell / slash still work; prompt chat reports this on submit.
            eprintln!("sui: LLM not configured ({error})");
        },
    }
    app.run_inline().map_err(MainError::from)
}
