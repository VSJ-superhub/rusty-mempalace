use anyhow::{Context, Result};
use yourmemory_core::storage::Palace;

const DEFAULT_CONFIG: &str = r#"[compression]
backend = "ollama"
model = "gemma:1b"

[palace]
schema = "general"
"#;

pub fn run() -> Result<()> {
    let cwd = std::env::current_dir().context("cannot determine current directory")?;
    let palace_dir = cwd.join(".yourmemory");

    if palace_dir.exists() {
        println!("Palace already exists at {}", palace_dir.display());
        return Ok(());
    }

    std::fs::create_dir_all(&palace_dir)
        .with_context(|| format!("cannot create {}", palace_dir.display()))?;

    let config_path = palace_dir.join("config.toml");
    std::fs::write(&config_path, DEFAULT_CONFIG)
        .with_context(|| format!("cannot write {}", config_path.display()))?;

    // Open (and migrate) the database.
    Palace::open(&cwd).context("cannot initialise Palace database")?;

    println!("Initialised Palace at {}", palace_dir.display());
    println!("Config written to {}", config_path.display());
    Ok(())
}
