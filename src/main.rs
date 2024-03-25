use std::{
    fmt::Display,
    fs::File,
    io::{stderr, stdout, BufReader, Read, Write},
    path::PathBuf,
    process::exit,
};

use clap::Parser;
use codemap::Codemap;
use parser::Span;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use vm::{VirtualMachine, VirtualMachineError};

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

impl App {
    fn report_error_with_span(
        code: &str,
        codemap: &Codemap,
        err_out: &mut impl Write,
        span: &Span,
        error: impl Display,
    ) -> std::io::Result<()> {
        writeln!(
            err_out,
            "{}Error at '{}': {}",
            {
                if let Some(line) = codemap.line_at(span.start()) {
                    format!("[line {}] ", line)
                } else {
                    String::from("")
                }
            },
            span.slice(code),
            error
        )
    }

    fn report_errors(
        &self,
        code: &str,
        codemap: &Codemap,
        err_out: &mut impl Write,
        vm_err: VirtualMachineError,
    ) -> std::io::Result<()> {
        match vm_err {
            VirtualMachineError::CompileError(parsing_errors) => {
                for error in parsing_errors.iter() {
                    match error {
                        parser::ParseError::VariableAlreadyDeclared { span } => {
                            Self::report_error_with_span(code, codemap, err_out, span, error)?
                        }
                        parser::ParseError::UnexpectedToken { span, .. } => {
                            Self::report_error_with_span(code, codemap, err_out, span, error)?
                        }
                        parser::ParseError::InvalidVariableName { span } => {
                            Self::report_error_with_span(code, codemap, err_out, span, error)?
                        }
                        parser::ParseError::InvalidAssignmentTarget { span } => {
                            Self::report_error_with_span(code, codemap, err_out, span, error)?
                        }
                        parser::ParseError::ExpectedExpression { span } => {
                            Self::report_error_with_span(code, codemap, err_out, span, error)?
                        }
                        parser::ParseError::MaxFunCallArguments { span } => {
                            Self::report_error_with_span(code, codemap, err_out, span, error)?
                        }
                        parser::ParseError::MaxFunDeclarationParameters { span } => {
                            Self::report_error_with_span(code, codemap, err_out, span, error)?
                        }
                        _ => writeln!(err_out, "{}", error)?,
                    }
                }
            }
            _ => writeln!(err_out, "{}", vm_err)?,
        }
        Ok(())
    }
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
        opts.report_errors(&code, &codemap, &mut stderr(), err)?;
        exit(65);
    }
    if opts.decompile {
        vm.decompile(&mut stdout())?;
        exit(0);
    }
    if let Err(err) = vm.run(&mut stdout()) {
        opts.report_errors(&code, &codemap, &mut stderr(), err)?;
        exit(70);
    }
    Ok(())
}
