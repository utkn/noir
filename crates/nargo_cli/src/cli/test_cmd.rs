use std::{collections::BTreeMap, io::Write, path::Path};

use acvm::{pwg::block::Blocks, PartialWitnessGenerator, ProofSystemCompiler, UnresolvedData};
use clap::Args;
use nargo::ops::execute_circuit;
use noirc_driver::{CompileOptions, Driver};
use noirc_frontend::node_interner::FuncId;
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

use crate::{errors::CliError, resolver::Resolver};

use super::NargoConfig;

/// Run the tests for this program
#[derive(Debug, Clone, Args)]
pub(crate) struct TestCommand {
    /// If given, only tests with names containing this string will be run
    test_name: Option<String>,

    #[clap(flatten)]
    compile_options: CompileOptions,
}

pub(crate) fn run(args: TestCommand, config: NargoConfig) -> Result<(), CliError> {
    let test_name: String = args.test_name.unwrap_or_else(|| "".to_owned());

    run_tests(&config.program_dir, &test_name, &args.compile_options)
}

fn run_tests(
    program_dir: &Path,
    test_name: &str,
    compile_options: &CompileOptions,
) -> Result<(), CliError> {
    let backend = crate::backends::ConcreteBackend;

    let mut driver = Resolver::resolve_root_manifest(program_dir, backend.np_language())?;

    driver.check_crate(compile_options).map_err(|_| CliError::CompilationError)?;

    let test_functions = driver.get_all_test_functions_in_crate_matching(test_name);
    println!("Running {} test functions...", test_functions.len());
    let mut failing = 0;

    let writer = StandardStream::stderr(ColorChoice::Always);
    let mut writer = writer.lock();

    for test_function in test_functions {
        let test_name = driver.function_name(test_function);
        writeln!(writer, "Testing {test_name}...").expect("Failed to write to stdout");
        writer.flush().ok();

        match run_test(test_name, test_function, &driver, compile_options) {
            Ok(_) => {
                writer.set_color(ColorSpec::new().set_fg(Some(Color::Green))).ok();
                writeln!(writer, "ok").ok();
            }
            // Assume an error was already printed to stdout
            Err(_) => failing += 1,
        }
        writer.reset().ok();
    }

    if failing == 0 {
        writer.set_color(ColorSpec::new().set_fg(Some(Color::Green))).unwrap();
        writeln!(writer, "All tests passed").ok();
    } else {
        let plural = if failing == 1 { "" } else { "s" };
        return Err(CliError::Generic(format!("{failing} test{plural} failed")));
    }

    writer.reset().ok();
    Ok(())
}

fn run_test(
    test_name: &str,
    main: FuncId,
    driver: &Driver,
    config: &CompileOptions,
) -> Result<(), CliError> {
    let backend = crate::backends::ConcreteBackend;

    let program = driver
        .compile_no_check(config, main)
        .map_err(|_| CliError::Generic(format!("Test '{test_name}' failed to compile")))?;
    let mut solved_witness = BTreeMap::new();
    let mut blocks = Blocks::default();
    
    // Run the backend to ensure the PWG evaluates functions like std::hash::pedersen,
    // otherwise constraints involving these expressions will not error.
    match backend.solve(&mut solved_witness, &mut blocks, program.circuit.opcodes) {
        Ok(UnresolvedData { unresolved_opcodes, unresolved_oracles, unresolved_brilligs }) => {
            if !unresolved_opcodes.is_empty()
                || !unresolved_oracles.is_empty()
                || !unresolved_brilligs.is_empty()
            {
                todo!("Add oracle support to nargo execute")
            }
            Ok(())
        }
        Err(error) => {
            let writer = StandardStream::stderr(ColorChoice::Always);
            let mut writer = writer.lock();
            writer.set_color(ColorSpec::new().set_fg(Some(Color::Red))).ok();
            writeln!(writer, "failed").ok();
            writer.reset().ok();
            Err(error.into())
        }
    }
}
