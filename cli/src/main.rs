mod init;
mod mine;
mod ops;
mod setup;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "yourmemory", about = "Local AI memory system")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initialise a Palace in the current directory.
    Init,
    /// Wire up the MCP server for an AI client.
    Setup {
        /// Target client: claude, gemini, or kiro.
        #[arg(value_enum)]
        client: setup::Client,
    },
    /// Index files in a directory into the Palace.
    Mine {
        /// Path to index.
        path: String,
    },
    /// Print recent Palace context (L0/L1 wakeup).
    Wakeup,
    /// Search the Palace for a query.
    Search {
        /// Query string.
        query: String,
    },
    /// Store a fact in the Palace.
    Persist {
        /// Text to store.
        text: String,
    },
    /// Print Palace statistics.
    Health,
    /// Run the forgetting/compaction pass.
    Compact,
}

fn main() {
    let cli = Cli::parse();
    let result = match &cli.command {
        Command::Init => init::run(),
        Command::Setup { client } => setup::run(client),
        Command::Mine { path } => mine::run(path),
        Command::Wakeup => ops::wakeup(),
        Command::Search { query } => ops::search(query),
        Command::Persist { text } => ops::persist(text),
        Command::Health => ops::health(),
        Command::Compact => ops::compact(),
    };
    if let Err(e) = result {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}
