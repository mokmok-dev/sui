//! Smoke: call `code_search` and `bash` through [`sui_tools::builtin_registry`].
//!
//! ```text
//! cargo run -p sui-tools --example smoke_builtin -- /path/to/repo
//! ```

use std::{env, path::PathBuf, sync::Arc, time::Duration};

use serde_json::json;
use sui_tools::{Bm25Index, ToolsError, builtin_registry};

#[tokio::main]
async fn main() -> Result<(), ToolsError> {
    let root = match env::args().nth(1) {
        Some(path) => PathBuf::from(path),
        None => env::current_dir().map_err(|source| ToolsError::io(".", source))?,
    };

    println!("indexing {} …", root.display());
    let index = Arc::new(Bm25Index::index_tree(&root, &["rs", "toml", "md", "rhai"])?);
    println!("indexed {} documents", index.len());

    let registry = builtin_registry(Arc::clone(&index), Some(&root))?;
    println!("tools: {:?}", registry.names());
    println!(
        "descriptors: {}",
        serde_json::to_string_pretty(&registry.descriptors())?
    );

    let search = registry
        .call(
            "code_search",
            json!({ "query": "Bm25Index builtin_registry", "limit": 5 }),
        )
        .await?;
    println!(
        "\n=== code_search ===\n{}",
        serde_json::to_string_pretty(&search)?
    );

    let hits = search["hits"]
        .as_array()
        .ok_or_else(|| ToolsError::Search("missing hits".into()))?;
    if hits.is_empty() {
        return Err(ToolsError::Search(
            "expected at least one hit for Bm25Index".into(),
        ));
    }

    let written = registry
        .call(
            "bash",
            json!({
                "command": "printf 'smoke-ok %s\\n' \"$(basename \"$PWD\")\"",
                "drain": false
            }),
        )
        .await?;
    println!(
        "\n=== bash write (drain:false) ===\n{}",
        serde_json::to_string_pretty(&written)?
    );

    tokio::time::sleep(Duration::from_millis(80)).await;
    let drained = registry.call("bash", json!({ "action": "drain" })).await?;
    println!(
        "\n=== bash drain ===\n{}",
        serde_json::to_string_pretty(&drained)?
    );
    let stdout = drained["stdout"].as_str().unwrap_or_default();
    if !stdout.contains("smoke-ok") {
        return Err(ToolsError::Bash(format!(
            "expected smoke-ok in stdout, got {stdout:?}"
        )));
    }

    let polled = registry.call("bash", json!({ "action": "poll" })).await?;
    println!(
        "\n=== bash poll ===\n{}",
        serde_json::to_string_pretty(&polled)?
    );

    let _ = registry
        .call("bash", json!({ "command": "exit", "drain": false }))
        .await?;
    let waited = registry
        .call("bash", json!({ "action": "wait", "timeout_ms": 3000 }))
        .await?;
    println!(
        "\n=== bash wait ===\n{}",
        serde_json::to_string_pretty(&waited)?
    );

    println!("\nsmoke_builtin: OK");
    Ok(())
}
