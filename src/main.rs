use std::{
    fs::File,
    io::{stderr, stdout, BufReader, Read, Write},
    path::PathBuf,
    process::exit,
};

use clap::Parser;
use codemap::Codemap;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use vm::VirtualMachine;

use crate::vm::DisplayError;

mod byte_code;
mod codemap;
mod interner;
mod lexer;
mod parser;
#[cfg(test)]
mod tests;
mod value;
mod vm;

#[derive(clap::Parser)]
struct App {
    #[arg(long)]
    decompile: bool,
    file: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let opts = App::parse();

    tracing_subscriber::registry()
        .with(fmt::Layer::default())
        .with(EnvFilter::from_default_env())
        .try_init()?;

    let mut code = String::new();
    BufReader::new(File::open(&opts.file)?).read_to_string(&mut code)?;
    let codemap = Codemap::new(&code);
    let mut vm = VirtualMachine::new();
    if let Err(err) = vm.compile(&code) {
        writeln!(
            &mut stderr(),
            "{}",
            DisplayError {
                code: &code,
                codemap: &codemap,
                err: &err
            }
        )?;
        exit(65);
    }
    if opts.decompile {
        vm.decompile(&mut stdout())?;
        exit(0);
    }
    if let Err(err) = vm.run(&mut stdout()) {
        writeln!(
            &mut stderr(),
            "{}",
            DisplayError {
                code: &code,
                codemap: &codemap,
                err: &err
            }
        )?;
        exit(70);
    }
    Ok(())
}
