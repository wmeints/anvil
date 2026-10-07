use std::env;

use anvil_cli::manage::OutputFormat;
use anvil_cli::{client, manage, secret, session, ssh, validate};
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
    Ls {
        /// Output format
        #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
        format: OutputFormat,
    },
    /// Remove a sandbox
    Rm,
    /// Run a command inside the sandbox
    Run(RunArgs),
    /// Validate the .anvil.yml file in the working directory
    Validate,
    /// Manage the secrets sandboxes use without seeing their values
    #[command(subcommand)]
    Secret(SecretCommands),
    /// Tunnel an SSH connection to a sandbox over stdin/stdout (used by the generated SSH config)
    #[command(hide = true)]
    SshProxy {
        /// Host name of the sandbox, e.g. `project.anvil`
        hostname: String,
    },
}

#[derive(Subcommand, Debug)]
enum SecretCommands {
    /// Set a secret for all sandboxes
    Set(SetSecretArgs),
    /// List the secrets and their allowed hosts, without their values
    Ls {
        /// Output format
        #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
        format: OutputFormat,
    },
    /// Remove a secret from all sandboxes
    Rm {
        /// Name of the secret, e.g. GH_TOKEN
        name: String,
    },
}

#[derive(Args, Debug)]
struct SetSecretArgs {
    /// Environment variable that exposes the secret in sandboxes, e.g. GH_TOKEN
    name: String,

    /// Value of the secret. Prefer --from-stdin to keep it out of your shell history
    #[arg(required_unless_present = "from_stdin", conflicts_with = "from_stdin")]
    value: Option<String>,

    /// Read the value from stdin
    #[arg(long)]
    from_stdin: bool,

    /// Host that may receive the value, e.g. api.example.com or *.example.com; repeat for more
    /// hosts. Defaults to the hosts of well-known secrets such as GH_TOKEN and ANTHROPIC_API_KEY
    #[arg(long = "allow-host", value_name = "HOST")]
    allowed_hosts: Vec<String>,
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

    // Read the secret before connecting, so a failing read doesn't start the daemon.
    if let Commands::Secret(SecretCommands::Set(args)) = cli.command {
        let value = secret_value(&args)?;
        let mut client_instance = client::connect().await?;

        return secret::set(args.name, value, args.allowed_hosts, &mut client_instance).await;
    }

    let mut client_instance = client::connect().await?;

    match cli.command {
        Commands::Start => manage::start_sandbox(&working_dir, &mut client_instance).await?,
        Commands::Stop => manage::stop_sandbox(&working_dir, &mut client_instance).await?,
        Commands::Ls { format } => {
            manage::list_sandboxes(&mut client_instance, format).await?;
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
        Commands::Secret(SecretCommands::Ls { format }) => {
            secret::list(format, &mut client_instance).await?;
        }
        Commands::Secret(SecretCommands::Rm { name }) => {
            secret::remove(name, &mut client_instance).await?;
        }
        Commands::Validate | Commands::Secret(SecretCommands::Set(_)) => {
            unreachable!("handled before connecting to the daemon")
        }
    }

    Ok(())
}

/// Returns the secret value from the arguments, or from stdin with `--from-stdin`.
fn secret_value(args: &SetSecretArgs) -> Result<String> {
    match &args.value {
        Some(value) => Ok(value.clone()),
        None => secret::read_value(std::io::stdin().lock()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parse_secret_set(args: &[&str]) -> Result<SetSecretArgs, clap::Error> {
        let cli = Cli::try_parse_from(["anvil", "secret", "set"].iter().chain(args))?;

        match cli.command {
            Commands::Secret(SecretCommands::Set(args)) => Ok(args),
            command => panic!("unexpected command {command:?}"),
        }
    }

    #[test]
    fn cli_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn secret_set_takes_value_and_hosts() {
        let args = parse_secret_set(&[
            "MY_TOKEN",
            "abc",
            "--allow-host",
            "a.example.com",
            "--allow-host",
            "*.example.org",
        ])
        .unwrap();

        assert_eq!(args.name, "MY_TOKEN");
        assert_eq!(args.value.as_deref(), Some("abc"));
        assert!(!args.from_stdin);
        assert_eq!(args.allowed_hosts, ["a.example.com", "*.example.org"]);
    }

    #[test]
    fn secret_set_takes_value_from_stdin() {
        let args = parse_secret_set(&["GH_TOKEN", "--from-stdin"]).unwrap();

        assert!(args.from_stdin);
        assert_eq!(args.value, None);
    }

    #[test]
    fn secret_ls_defaults_to_table() {
        let cli = Cli::try_parse_from(["anvil", "secret", "ls"]).unwrap();

        assert!(matches!(
            cli.command,
            Commands::Secret(SecretCommands::Ls {
                format: OutputFormat::Table
            })
        ));
    }

    #[test]
    fn secret_rm_requires_name() {
        assert!(Cli::try_parse_from(["anvil", "secret", "rm"]).is_err());

        let cli = Cli::try_parse_from(["anvil", "secret", "rm", "GH_TOKEN"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Secret(SecretCommands::Rm { name }) if name == "GH_TOKEN"
        ));
    }

    #[test]
    fn secret_set_requires_exactly_one_value_source() {
        assert!(parse_secret_set(&["GH_TOKEN"]).is_err());
        assert!(parse_secret_set(&["GH_TOKEN", "abc", "--from-stdin"]).is_err());
    }
}
