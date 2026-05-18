use std::{
    fs::File,
    io::{stderr, stdout, BufReader, Read, Write},
    path::PathBuf,
    process::exit,
};

use clap::Parser;
use codemap::Codemap;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use vm::{BeforeStmtAction, VirtualMachine};

use crate::vm::DisplayError;

mod byte_code;
mod codemap;
mod gc;
mod interner;
mod lexer;
mod object;
mod parser;
mod stats;
#[cfg(test)]
mod tests;
mod value;
mod vm;

#[derive(clap::Parser)]
struct App {
    #[arg(long)]
    decompile: bool,
    #[arg(long, num_args = 0..=1, default_missing_value = "", require_equals = true)]
    stats: Option<String>,
    /// Print `[line N]` to stderr before the first opcode of every new
    /// source line. Useful for understanding control flow and pinpointing
    /// where the VM spends time. Built on the `BeforeStmt` VM hook.
    #[arg(long)]
    trace: bool,
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
    if opts.stats.is_some() {
        vm.enable_stats();
    }
    if opts.trace {
        vm.set_before_stmt(Some(Box::new(|vm: &mut VirtualMachine| {
            eprintln!("[line {}]", vm.current_line());
            BeforeStmtAction::Continue
        })));
    }
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
    if let Some(stats_path) = &opts.stats {
        if let Some(counters) = vm.take_counters() {
            if stats_path.is_empty() {
                counters.print_summary(&mut stderr())?;
            } else {
                counters.write_json(std::path::Path::new(stats_path))?;
            }
        }
    }
    Ok(())
}
