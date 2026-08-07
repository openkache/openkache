#[cfg(all(feature = "quic-compio", feature = "quic-quinn"))]
compile_error!("enable exactly one CLI QUIC backend feature");

#[cfg(not(any(feature = "quic-compio", feature = "quic-quinn")))]
compile_error!("enable one CLI QUIC backend feature");

#[cfg(feature = "quic-compio")]
fn main() {
    use clap::Parser;

    let arguments = openkache_cli::Arguments::parse();
    let runtime = match compio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            openkache_cli::report_message(
                format!("failed to initialize Compio runtime: {error}"),
                "check that the selected runtime is available on this host",
            );
            std::process::exit(1);
        }
    };
    if !runtime.driver_type().is_iouring() {
        openkache_cli::report_message(
            "openkache-cli requires the io_uring runtime driver",
            "use `--no-default-features --features quic-quinn` on platforms without io_uring",
        );
        std::process::exit(1);
    }
    if let Err(error) = runtime.block_on(openkache_cli::run(arguments)) {
        openkache_cli::report_error(&error);
        std::process::exit(1);
    }
}

#[cfg(feature = "quic-quinn")]
#[tokio::main]
async fn main() {
    use clap::Parser;

    if let Err(error) = openkache_cli::run(openkache_cli::Arguments::parse()).await {
        openkache_cli::report_error(&error);
        std::process::exit(1);
    }
}
