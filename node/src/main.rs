//! Substrate Node Template CLI library.
#![warn(missing_docs)]

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;
mod chain_spec;
mod cli;
mod command;
mod rpc;
mod service;
#[cfg(feature = "runtime-benchmarks")]
mod zklogin_benchmarking;
fn main() -> sc_cli::Result<()> {
    command::run()
}
