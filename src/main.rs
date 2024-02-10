use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt};
use vm::VirtualMachine;

mod byte_code;
mod interner;
mod lexer;
mod parser;
mod value;
mod vm;

const ASSIGNMENT: &str = r#"
var var1 = "hello";
print var1 + " world";
var1 = "bye-bye";
print var1;
"#;

const UNDECLARED_ASSIGNMENT: &str = r#"
var1 = "test";
"#;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(fmt::Layer::default())
        //.with(EnvFilter::from_default_env())
        .try_init()?;

    let mut vm = VirtualMachine::new();

    //vm.interpret(UNDECLARED_ASSIGNMENT)?;
    vm.interpret(ASSIGNMENT)?;
    Ok(())
}
