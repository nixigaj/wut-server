#[macro_use]
extern crate log;

use anyhow::Result;
use clap::{arg, Parser};
use hyper::server::conn::AddrIncoming;
use hyper::server::Builder;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Request, Response, Server};
use hyper_rustls::acceptor::TlsStream;
use hyper_rustls::TlsAcceptor;
use std::convert::Infallible;
use std::io::{Seek, SeekFrom};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::vec::Vec;
use std::{env, fs, io};
use tokio::signal::unix::{signal, SignalKind};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio::{select, time};

const DEFAULT_PORT: u16 = 11313;

/// A HTTPS server that echoes the client's IP-address
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
	/// Address to bind to, with optional port (can be provided multiple times)
	#[arg(short, long, default_values = vec!["127.0.0.1:11313", "[::1]:11313"])]
	bind: Vec<String>,

	/// Certificate file path
	#[arg(short, long, required = true)]
	cert_path: String,

	/// Key file path
	#[arg(short, long, required = true)]
	key_path: String,

	/// Log interval in seconds
	#[arg(short = 'i', long = "log-interval", default_value_t = 60)]
	log_interval: u64,

	/// Use HTTP/2 only
	#[arg(short = '2', long = "http2-only", default_value_t = false)]
	http2_only: bool,
}

pub fn main() {
	match env::var("RUST_LOG") {
		Err(_) => {
			env::set_var("RUST_LOG", "info");
		}
		Ok(_) => {}
	}

	env_logger::init();

	let args = Args::parse();

	if let Err(e) = run_server(args) {
		error!("Fatal: {}", e);
		std::process::exit(1);
	}
}

#[tokio::main]
async fn run_server(args: Args) -> Result<()> {
	let certs = load_certs(args.cert_path.as_str())?;
	let key = load_private_key(args.key_path.as_str())?;

	let mut servers: Vec<Builder<TlsAcceptor>> = Vec::new();

	for bind in &args.bind {
		let addr = parse_addr(bind)?;

		let incoming = AddrIncoming::bind(&addr)?;
		let acceptor = TlsAcceptor::builder()
			.with_single_cert(certs.clone(), key.clone())
			.map_err(|e| error(format!("{}", e)))?
			.with_all_versions_alpn()
			.with_incoming(incoming);

		let server = Server::builder(acceptor).http2_only(args.http2_only);

		servers.push(server);

		if addr.is_ipv4() {
			info!("Starting to serve IPv4 on https://{addr}",);
		} else {
			info!("Starting to serve IPv6 on https://{addr}",);
		}
	}

	let req_counter = AtomicU64::new(0);
	let req_counter_arc = Arc::new(req_counter);
	let req_counter_arc_service = req_counter_arc.clone();

	let service = make_service_fn(move |socket: &TlsStream| {
		req_counter_arc_service.fetch_add(1, Ordering::SeqCst);

		let conn = socket.io();
		let remote_addr: String;

		match conn {
			None => remote_addr = String::from("error"),
			Some(val) => remote_addr = format!("{}", val.remote_addr().ip()),
		}

		async move {
			Ok::<_, Infallible>(service_fn(move |_: Request<Body>| {
				let response = Response::new(Body::from(remote_addr.clone()));
				async { Ok::<_, Infallible>(response) }
			}))
		}
	});

	let mut server_handles: Vec<JoinHandle<hyper::Result<()>>> = Vec::new();

	for server in servers {
		server_handles.push(tokio::spawn(
			server
				.serve(service.clone())
				.with_graceful_shutdown(server_shutdown_signal()),
		));
	}
	info!("Server started");

	start_counter(args.log_interval, req_counter_arc).await;

	for server_handle in server_handles {
		server_handle.await?.unwrap();
	}

	Ok(())
}

async fn start_counter(log_interval: u64, req_counter_arc: Arc<AtomicU64>) {
	let start_time = Instant::now();
	let mut prev_elapsed_time = Duration::new(0, 0);
	let mut prev_total_requests = 0;
	let mut interval = time::interval(Duration::from_secs(log_interval));
	interval.tick().await;

	loop {
		select! {
			_ = interval.tick() => {},
			exit_type = server_shutdown_counter() => {
				match exit_type {
					ExitType::Interrupt => {
						info!("Received interrupt signal. Exiting...");
					}
					ExitType::Termination => {
						info!("Received termination signal. Exiting...");
					}
				}
				break;
			},
		}
		let total_requests = req_counter_arc.load(Ordering::Relaxed);
		let total_requests_diff = total_requests - prev_total_requests;
		let elapsed_time = start_time.elapsed() - prev_elapsed_time;
		let rps = total_requests_diff as f64 / elapsed_time.as_secs() as f64;
		let rps_tot = total_requests as f64 / start_time.elapsed().as_secs() as f64;

		info!(
			"\nRequests per second: {:.2}\nTotal requests per second: {:.2}\nTotal requests: {}",
			rps, rps_tot, total_requests
		);

		prev_elapsed_time = elapsed_time;
		prev_total_requests = total_requests;
	}
}

fn error(err: String) -> io::Error {
	io::Error::new(io::ErrorKind::Other, err)
}

fn load_certs(filename: &str) -> io::Result<Vec<rustls::Certificate>> {
	let cert_file = fs::File::open(filename)
		.map_err(|e| error(format!("failed to open {}: {}", filename, e)))?;
	let mut reader = io::BufReader::new(cert_file);

	let certs = rustls_pemfile::certs(&mut reader)
		.map_err(|_| error("failed to load certificate".into()))?;
	Ok(certs.into_iter().map(rustls::Certificate).collect())
}

fn load_private_key(filename: &str) -> io::Result<rustls::PrivateKey> {
	let keyfile = fs::File::open(filename)
		.map_err(|e| error(format!("failed to open {}: {}", filename, e)))?;
	let mut reader = io::BufReader::new(keyfile);

	let ec_keys = {
		reader.seek(SeekFrom::Start(0))?;
		rustls_pemfile::ec_private_keys(&mut reader)
			.map_err(|_| error("failed to read EC private keys".into()))?
	};

	let pkcs8_keys = {
		reader.seek(SeekFrom::Start(0))?;
		rustls_pemfile::pkcs8_private_keys(&mut reader)
			.map_err(|_| error("failed to read PKCS8 private keys".into()))?
	};

	let rsa_keys = {
		reader.seek(SeekFrom::Start(0))?;
		rustls_pemfile::rsa_private_keys(&mut reader)
			.map_err(|_| error("failed to read RSA private keys".into()))?
	};

	let total_keys = ec_keys.len() + pkcs8_keys.len() + rsa_keys.len();

	match (
		ec_keys.first(),
		pkcs8_keys.first(),
		rsa_keys.first(),
		total_keys,
	) {
		(Some(ec_key), _, _, 1) => Ok(rustls::PrivateKey(ec_key.clone())),
		(_, Some(pkcs8_key), _, 1) => Ok(rustls::PrivateKey(pkcs8_key.clone())),
		(_, _, Some(rsa_key), 1) => Ok(rustls::PrivateKey(rsa_key.clone())),
		(_, _, _, 0) => Err(error(format!("no private keys found in file {}", filename)).into()),
		_ => Err(error(format!(
			"expected a single private key in file {}",
			filename
		))
		.into()),
	}
}

fn parse_addr(bind: &String) -> Result<SocketAddr> {
	// The user tried to enter an IPv4 or IPv6 with
	// a port and the address should be parsed as is.
	if bind.matches(":").count() == 1 || bind.as_str().contains("]:") {
		return match bind.parse() {
			Ok(addr) => Ok(addr),
			_ => Err(anyhow::Error::msg(format!(
				"failed to parse bind IP address including port: {}",
				bind
			))),
		};
	}

	// Test IPv4 first
	return match format!("{bind}:{DEFAULT_PORT}").parse() {
		Ok(addr) => Ok(addr),
		_ => {
			// Then IPv6
			match format!("[{bind}]:{DEFAULT_PORT}").parse() {
				Ok(addr) => Ok(addr),
				_ => {
					return Err(anyhow::Error::msg(format!(
						"failed to parse bind IP address: {}",
						bind
					)))
				}
			}
		}
	};
}

enum ExitType {
	Termination,
	Interrupt,
}

async fn shutdown_signal_helper() -> ExitType {
	let mut sigterm
		= signal(SignalKind::terminate())
		.expect("failed to initialize SIGTERM handler");

	let mut sigint
		= signal(SignalKind::interrupt())
		.expect("failed to initialize SIGTERM handler");

	select! {
		_ = sigterm.recv() => return ExitType::Termination,
		_ = sigint.recv() => return ExitType::Interrupt,
	}
}

async fn server_shutdown_signal() {
	let _ = shutdown_signal_helper().await;
}

async fn server_shutdown_counter() -> ExitType {
	return shutdown_signal_helper().await;
}
