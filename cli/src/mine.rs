use anyhow::{Context, Result};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use yourmemory_core::storage::{Confidence, NewDrawer, Palace, Source, Storage};

const SKIP_DIRS: &[&str] = &[
    ".git", ".hg", ".svn",
    "node_modules", "__pycache__",
    "venv", ".venv", "env", ".env",
    "target", "dist", "build",
    ".next", ".nuxt",
    "coverage", ".pytest_cache", ".mypy_cache", ".ruff_cache",
    ".tox", "out", "eggs", ".eggs", "htmlcov", ".cache",
    ".yourmemory",
];

const ALLOWED_EXTENSIONS: &[&str] = &[
    "py", "ts", "tsx", "js", "jsx", "rs", "go", "java", "c", "cpp",
    "h", "hpp", "cs", "rb", "php", "swift", "kt", "scala", "r",
    "md", "txt", "toml", "yaml", "yml", "json",
    "html", "css", "scss", "sql",
    "sh", "bash", "zsh", "fish",
    "tf", "hcl",
];

const EXTENSIONLESS_NAMES: &[&str] = &[
    "dockerfile", "makefile", "rakefile", "gemfile",
    "procfile", "vagrantfile", "jenkinsfile",
];

const MAX_BYTES: usize = 512 * 1024;

fn is_likely_binary(data: &[u8]) -> bool {
    let sample = &data[..data.len().min(8192)];
    if sample.contains(&0u8) {
        return true;
    }
    let non_text = sample
        .iter()
        .filter(|&&b| b < 9 || (14 <= b && b < 32) || b == 127)
        .count();
    non_text as f64 / sample.len().max(1) as f64 > 0.10
}

fn collect_paths(root: &Path) -> Vec<PathBuf> {
    WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.file_type().is_dir() {
                let name = e.file_name().to_str().unwrap_or("");
                if e.depth() == 0 {
                    return true;
                }
                return !SKIP_DIRS.contains(&name) && !name.starts_with('.');
            }
            true
        })
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .collect()
}

fn read_files(paths: &[PathBuf], root: &Path) -> Vec<(String, String)> {
    paths
        .par_iter()
        .filter_map(|path| {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();

            let allowed = if !ext.is_empty() {
                ALLOWED_EXTENSIONS.contains(&ext.as_str())
            } else {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                EXTENSIONLESS_NAMES.contains(&name.as_str())
            };
            if !allowed {
                return None;
            }

            let size = std::fs::metadata(path).ok()?.len() as usize;
            if size == 0 || size > MAX_BYTES {
                return None;
            }

            let data = std::fs::read(path).ok()?;
            if is_likely_binary(&data) {
                return None;
            }

            let content = String::from_utf8_lossy(&data).into_owned();
            let rel = path
                .strip_prefix(root)
                .ok()
                .map(|p| p.to_string_lossy().replace('\\', "/"))?;

            Some((rel, content))
        })
        .collect()
}

pub fn run(path: &str) -> Result<()> {
    let target = Path::new(path)
        .canonicalize()
        .with_context(|| format!("cannot resolve path: {path}"))?;

    println!("Walking {}…", target.display());

    let paths = collect_paths(&target);
    println!("Found {} candidate files", paths.len());

    let files = read_files(&paths, &target);
    println!("Read {} text files", files.len());

    let cwd = std::env::current_dir().context("cannot determine current directory")?;
    let palace = Palace::open(&cwd).context("Palace not initialised — run `yourmemory init` first")?;

    let wing = palace
        .storage
        .create_wing("mine", Some("files indexed by yourmemory mine"))
        .context("cannot create wing")?;

    let mut stored = 0usize;
    for (rel, content) in &files {
        // Use the file path as the room name (truncated to a reasonable length).
        let room_name: String = rel.chars().take(200).collect();
        let room = palace
            .storage
            .create_room(wing.id, &room_name, None)
            .context("cannot create room")?;

        palace
            .storage
            .store_drawer(&NewDrawer {
                wing_id: wing.id,
                room_id: room.id,
                content: content.clone(),
                compressed_content: None,
                confidence: Confidence::Medium,
                source: Source::System,
            })
            .context("cannot store drawer")?;

        stored += 1;
    }

    println!("Stored {stored} drawers.");
    Ok(())
}
