use clap::{Parser, Subcommand};
use anyhow::Result;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Connects to a remote VPS and executes a command
    Exec {
        /// The target VPS host (e.g., user@host)
        host: String,
        /// The command to execute on the remote VPS
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Uploads a file to a remote VPS
    Upload {
        /// The target VPS host (e.g., user@host)
        host: String,
        /// Local path to the file to upload
        local_path: String,
        /// Remote path where the file will be uploaded
        remote_path: String,
    },
    /// Downloads a file from a remote VPS
    Download {
        /// The target VPS host (e.g., user@host)
        host: String,
        /// Remote path to the file to download
        remote_path: String,
        /// Local path where the file will be saved
        local_path: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Exec { host, command } => {
            println!("Executing command {:?} on host {}", command, host);
            // TODO: Implement SSH execution logic using russh
        }
        Commands::Upload { host, local_path, remote_path } => {
            println!("Uploading {} to {} on host {}", local_path, remote_path, host);
            // TODO: Implement SSH upload logic using russh
        }
        Commands::Download { host, remote_path, local_path } => {
            println!("Downloading {} from {} on host {}", remote_path, local_path, host);
            // TODO: Implement SSH download logic using russh
        }
    }

    Ok(())
}
