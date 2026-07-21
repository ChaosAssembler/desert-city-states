use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "dcs-app")]
#[command(about = "Desert City-States application")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the game server
    Serve,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Serve => {
            if let Err(e) = dcs_app::serve::run_serve() {
                eprintln!("Server error: {e}");
                std::process::exit(1);
            }
        }
    }
}
