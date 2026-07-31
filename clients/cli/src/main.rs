#[cfg(all(feature = "quic-compio", feature = "quic-quinn"))]
compile_error!("enable exactly one CLI QUIC backend feature");

#[cfg(not(any(feature = "quic-compio", feature = "quic-quinn")))]
compile_error!("enable one CLI QUIC backend feature");

#[cfg(feature = "quic-compio")]
fn main() {
    use clap::Parser;

    let runtime = match compio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("ERR failed to initialize Compio runtime: {error}");
            std::process::exit(1);
        }
    };
    if !runtime.driver_type().is_iouring() {
        eprintln!("ERR openkache-cli requires the io_uring runtime driver");
        std::process::exit(1);
    }
    if let Err(error) = runtime.block_on(openkache_cli::run(openkache_cli::Arguments::parse())) {
        eprintln!("ERR {error}");
        std::process::exit(1);
    }
}

#[cfg(feature = "quic-quinn")]
#[tokio::main]
async fn main() {
    use clap::Parser;

    if let Err(error) = openkache_cli::run(openkache_cli::Arguments::parse()).await {
        eprintln!("ERR {error}");
        std::process::exit(1);
    }
}
