use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt};
use vm::VirtualMachine;

mod byte_code;
mod lexer;
mod parser;
mod value;
mod vm;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(fmt::Layer::default())
        //.with(EnvFilter::from_default_env())
        .try_init()?;

    let mut vm = VirtualMachine::new();
    vm.interpret("!(10>2)")?;
    Ok(())
}
