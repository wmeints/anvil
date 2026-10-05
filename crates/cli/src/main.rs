use std::env;

use anvil_cli::{client, manage, session};
use anyhow::Result;
use clap::{Args, Parser, Subcommand};

/// Anvil - Run coding agents safely in a sandbox.
#[derive(Parser, Debug)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Start a new sandbox
    Start,
    /// Stop a running sandbox
    Stop,
    /// List all sandboxes
    Ls,
    /// Remove a sandbox
    Rm,
    /// Run a command inside the sandbox
    Run(RunArgs),
}

#[derive(Args, Debug)]
struct RunArgs {
    /// Command to execute in the session
    command: String,

    /// Arguments for the command
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut client_instance = client::connect().await?;
    let working_dir = env::current_dir()?;

    let cli = Cli::parse();

    match cli.command {
        Commands::Start => manage::start_sandbox(&working_dir, &mut client_instance).await?,
        Commands::Stop => manage::stop_sandbox(&working_dir, &mut client_instance).await?,
        Commands::Ls => {
            manage::list_sandboxes(&mut client_instance).await?;
        }
        Commands::Rm => manage::remove_sandbox(&working_dir, &mut client_instance).await?,
        Commands::Run(run_args) => {
            let code = session::attach(
                working_dir,
                run_args.command,
                run_args.args,
                &mut client_instance,
            )
            .await?;

            std::process::exit(code);
        }
    }

    Ok(())
}
