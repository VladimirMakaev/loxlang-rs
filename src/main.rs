use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt};

mod vm;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(fmt::Layer::default())
        //.with(EnvFilter::from_default_env())
        .try_init()?;

    let mut vm = vm::VirtualMachine::new(vec![0, 0, 0, 1, 1]);
    vm.add_constant(1.0.into());
    vm.add_constant(2.0.into());
    Ok(vm.run()?)
}
