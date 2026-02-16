use clap::{Parser, Subcommand};

mod index;
mod query;

#[derive(Parser)]
struct Cli {
    #[clap(subcommand)]
    command: CliCommand,
}

#[derive(Subcommand)]
enum CliCommand {
    Index(index::Cli),
    Query(query::Cli),
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    initialise_logging(&cli);

    match cli.command {
        CliCommand::Index(cli) => index::run(cli)?,
        CliCommand::Query(cli) => query::run(cli)?,
    }

    Ok(())
}

fn initialise_logging(cli: &Cli) {
    let log_level = match &cli.command {
        CliCommand::Index(cli) => cli.log_level,
        CliCommand::Query(cli) => cli.log_level,
    };

    use simplelog::*;
    CombinedLogger::init(vec![TermLogger::new(
        log_level,
        Config::default(),
        TerminalMode::Stdout,
        ColorChoice::Auto,
    )])
    .unwrap();
}
