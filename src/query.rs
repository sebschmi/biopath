use clap::Parser;
use log::LevelFilter;

#[derive(Parser)]
pub struct Cli {
    #[clap(long, default_value = "info")]
    pub(crate) log_level: LevelFilter,
}

pub fn run(_cli: Cli) -> anyhow::Result<()> {
    todo!()
}
