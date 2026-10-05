use std::env;

use anvil_cli::{client, manage, session, ssh, validate};
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
    /// Validate the .anvil.yml file in the working directory
    Validate,
    /// Tunnel an SSH connection to a sandbox over stdin/stdout (used by the generated SSH config)
    #[command(hide = true)]
    SshProxy {
        /// Host name of the sandbox, e.g. `project.anvil`
        hostname: String,
    },
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
    let cli = Cli::parse();
    let working_dir = env::current_dir()?;

    // Validating the spec doesn't need the daemon.
    if let Commands::Validate = cli.command {
        let valid = validate::validate_spec(&working_dir)?;
        std::process::exit(if valid { 0 } else { 1 });
    }

    let mut client_instance = client::connect().await?;

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
        Commands::SshProxy { hostname } => {
            ssh::proxy(hostname, &mut client_instance).await?;

            // Exit right away rather than wait for the stdin thread, which blocks on a read.
            std::process::exit(0);
        }
        Commands::Validate => unreachable!("handled before connecting to the daemon"),
    }

    Ok(())
}
