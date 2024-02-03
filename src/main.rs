use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt};

mod vm;
mod lexer;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(fmt::Layer::default())
        //.with(EnvFilter::from_default_env())
        .try_init()?;

    let mut vm = vm::VirtualMachine::new(vec![
        1, 0, // read 0
        1, 1, // read 1
        2, // add
        6,
    ]);
    vm.add_constant(1.0.into());
    vm.add_constant(2.0.into());
    Ok(vm.run()?)
}
