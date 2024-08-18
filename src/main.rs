mod cloudwatch;

extern crate dotenv;
use aws_sdk_cloudwatch::Client;
use chrono::Local;
use clap::Parser;
use daemonize::Daemonize;
use dotenv::dotenv;
use std::fs::{create_dir_all, File};
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use tokio::runtime::Runtime;
use tokio::time::sleep;
use tokio::time::Duration;
use tracing::Level;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use which::which;

pub const DAEMON_FILE_PATH: &str = "/tmp/ore_miner";
pub const STANDALONE_BINARY_NAME: &str = "ore";

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(long, help = "Number of CPU cores to use")]
    cores: String,
    #[clap(long, help = "Path to your keypair file")]
    keypair: String,
    #[clap(long, help = "Path to your fee payer file")]
    fee_payer: String,
    #[clap(long, help = "Enable dynamic fees")]
    dynamic_fee: bool,
    #[clap(long, help = "URL to your dynamic fee RPC")]
    dynamic_fee_url: String,
    #[clap(long, help = "URL to your RPC")]
    rpc: String,
    #[clap(
        long,
        help = "Path to the ore cli binary. Will default to \"ore\".",
        default_value = STANDALONE_BINARY_NAME
    )]
    ore_binary_path: String,
}

fn ensure_dir_exists(path: &str) {
    let path = Path::new(path);
    if !path.exists() {
        match create_dir_all(path) {
            Ok(_) => tracing::info!("Directory created: {:?}", path),
            Err(e) => eprintln!("Failed to create directory: {}", e),
        }
    }
}

fn is_process_running(pid: i32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .is_ok()
}

fn stop_daemon(pid_file_path: &str) -> Result<(), std::io::Error> {
    let mut file = File::open(pid_file_path)?;
    let mut pid = String::new();
    file.read_to_string(&mut pid)?;
    let pid: i32 = pid
        .trim()
        .parse()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    // send SIGTERM to the process
    if let Err(e) = Command::new("kill").arg(pid.to_string()).status() {
        eprintln!("Failed to stop daemon: {}", e);
    } else {
        tracing::info!("Daemon stopped successfully");
    }

    std::fs::remove_file(pid_file_path)?;

    Ok(())
}

///
/// tracing statements propagate to the log files. println statements also propagate to stdout for the user
/// up until the daemon is started. then, stdout/stderr is redirected to the log files.
///
fn main() {
    let file_appender = RollingFileAppender::new(Rotation::DAILY, "/var/log", "ore-miner.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    fmt()
        .with_writer(std::io::stdout)
        .with_writer(non_blocking.with_min_level(Level::INFO))
        .init();

    let args = Args::parse();
    dotenv().ok();

    ensure_dir_exists(DAEMON_FILE_PATH);

    // print the binary path if the default
    let binary = args.ore_binary_path.as_str();
    if binary.eq(STANDALONE_BINARY_NAME) {
        tracing::info!(
            "No custom binary provided, checking that the default binary = {} is in the system's path",
            binary
        );

        match which(binary) {
            Ok(path) => {
                tracing::info!("{} is located at: {}", binary, path.display());
            }
            Err(e) => {
                tracing::error!("{:?} not found in PATH: {:?}", binary, e);

                // alert the user based on stderr and then exit the program
                eprintln!(
                    "\n[Daemonization Failed] Unable to find the default {:?} binary in the system path. Make sure it's in the path or provide the path override with the \"ore_binary_path\" arg, and then try again.",
                    binary
                );

                return;
            }
        }
    }

    let pid_file_path = format!("{}/process.pid", DAEMON_FILE_PATH); // Check if the daemon is already running
    if let Ok(mut file) = File::open(&pid_file_path) {
        let mut pid = String::new();
        if file.read_to_string(&mut pid).is_ok() {
            if let Ok(pid) = pid.trim().parse::<i32>() {
                if is_process_running(pid) {
                    tracing::info!("Daemon is already running. Stopping it first.");
                    if let Err(e) = stop_daemon(&pid_file_path) {
                        eprintln!(
                            "\n[Daemonization Failed] Unable to stop existing daemon: {}",
                            e
                        );

                        return;
                    }
                } else {
                    // PID file exists but process is not running, remove the stale PID file
                    if let Err(e) = std::fs::remove_file(&pid_file_path) {
                        eprintln!(
                            "\n[Daemonization Failed] Unable to remove stale PID file: {}",
                            e
                        );
                        return;
                    }
                }
            }
        }
    }

    let stdout = File::create(format!("{}/daemon.out", DAEMON_FILE_PATH)).unwrap();
    let stderr = File::create(format!("{}/daemon.err", DAEMON_FILE_PATH)).unwrap();

    println!("Starting daemon... Check logs for subsequent output.");
    let daemonize = Daemonize::new()
        .pid_file(pid_file_path)
        .chown_pid_file(true)
        .working_directory("/tmp")
        .user(std::env::var("USER").as_deref().unwrap_or("root"))
        .group("daemon")
        .umask(0o777)
        .stdout(stdout)
        .stderr(stderr)
        .privileged_action(|| "Daemon privileged action");

    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    match daemonize.start() {
        Ok(_) => {
            tracing::info!("[{:?}] Daemon started", timestamp);
            println!("[{:?}] Daemon started", timestamp);
        }
        Err(e) => {
            tracing::error!("[{:?}] Error, {}", timestamp, e);
            eprintln!("\n[Daemonization Failed] Unable to start daemon");
            return;
        }
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    // runtime.block_on(async_main(args));
    runtime.block_on(async_main_test());
}

async fn process_output(line: &str, client: &Client) {
    tracing::info!("processing line: {}", line);
    match cloudwatch::process_mining_metrics(client, line).await {
        Ok(_) => tracing::info!("Successfully sent metrics to CloudWatch"),
        Err(e) => tracing::error!("Error: {:?}", e),
    }
}

async fn async_main_test() {
    let mut count = 0;
    loop {
        tracing::info!("Count: {}", count);
        sleep(Duration::from_secs(1)).await;
        count += 1;
    }
}

async fn async_main(args: Args) {
    let client = cloudwatch::create_cloudwatch_client().await;

    tracing::info!("path for ore binary: {}", &args.ore_binary_path);
    let mut binding = Command::new(&args.ore_binary_path);
    let mut command = binding
        .arg("mine")
        .arg("--cores")
        .arg(&args.cores)
        .arg("--keypair")
        .arg(&args.keypair)
        .arg("--rpc")
        .arg(&args.rpc);

    if !args.fee_payer.is_empty() {
        command = command.arg("--fee-payer").arg(&args.fee_payer);
    }

    if args.dynamic_fee {
        command = command.arg("--dynamic-fee");
    }

    if !args.dynamic_fee_url.is_empty() {
        command = command.arg("--dynamic-fee-url").arg(&args.dynamic_fee_url);
    }

    tracing::info!("command: {:?}", command);

    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start CLI tool");

    tracing::info!("ORE mining started");

    let stdout = child.stdout.take().expect("Failed to capture stdout");
    let stderr = child.stderr.take().expect("Failed to capture stderr");

    // read stdout in a separate thread
    let cloudwatch_client = client.clone();
    std::thread::spawn(move || {
        let rt: Runtime = Runtime::new().unwrap();
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            if let Ok(line) = line {
                rt.block_on(process_output(&line, &cloudwatch_client));
            }
        }
    });

    // read stderr in the main thread
    let stderr_reader = BufReader::new(stderr);
    for line in stderr_reader.lines() {
        if let Ok(line) = line {
            tracing::error!("CLI tool stderr: {:?}", line);
        }
    }

    // wait for the child process to exit
    let status = child.wait().expect("Failed to wait on child");
    tracing::info!("CLI tool exited with status: {}", status);
}
