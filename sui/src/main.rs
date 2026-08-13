use sui_app::App;
use sui_llm::LlmClient;
use sui_theme::config;
use sui_tools::{Bm25Index, coding_registry};
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

    let mut app = App::new().with_theme(config::load_active());
    match LlmClient::from_env() {
        Ok(client) => {
            let cwd = std::env::current_dir().ok();
            let index = cwd
                .as_ref()
                .and_then(|path| Bm25Index::index_tree(path, &["rs", "toml", "md", "rhai"]).ok())
                .unwrap_or_default();
            app = app
                .with_llm(client)
                .with_tools(coding_registry(index, cwd.as_deref()));
        },
        Err(error) => {
            // Shell / slash still work; prompt chat reports this on submit.
            eprintln!("sui: LLM not configured ({error})");
        },
    }
    app.run_inline().map_err(MainError::from)
}
