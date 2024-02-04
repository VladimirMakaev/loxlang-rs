use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt};
use vm::VirtualMachine;

mod lexer;
mod parser;
mod vm;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(fmt::Layer::default())
        //.with(EnvFilter::from_default_env())
        .try_init()?;

    let mut vm = VirtualMachine::new();
    vm.interpret("123+3")?;
    Ok(())
}
