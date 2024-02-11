use std::{
    fs::File,
    io::{stdout, BufReader, Read},
    path::PathBuf,
};

use clap::Parser;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use vm::VirtualMachine;

mod byte_code;
mod interner;
mod lexer;
mod parser;
#[cfg(test)]
mod tests;
mod value;
mod vm;

#[derive(clap::Parser)]
struct Opts {
    file: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let opts = Opts::parse();

    tracing_subscriber::registry()
        .with(fmt::Layer::default())
        .with(EnvFilter::from_default_env())
        .try_init()?;

    let mut code = String::new();
    BufReader::new(File::open(opts.file)?).read_to_string(&mut code)?;
    let mut vm = VirtualMachine::new();
    vm.compile(&code)?;
    vm.run(&mut stdout())?;
    Ok(())
}
