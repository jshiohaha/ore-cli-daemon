mod cloudwatch;
mod errors;

extern crate dotenv;
use aws_sdk_cloudwatch::Client;
use chrono::Local;
use clap::Parser;
use daemonize::Daemonize;
use dotenv::dotenv;
use std::env;
use std::fs::{create_dir_all, remove_file, File};
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread::JoinHandle;
use tokio::runtime::Runtime;
use tokio::time::sleep;
use tokio::time::Duration;
use tracing::Level;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use which::which;

use errors::{OreMinerError, Result};

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

fn ensure_dir_exists(path: &str) -> Result<()> {
    let path = Path::new(path);
    if !path.exists() {
        match create_dir_all(path) {
            Ok(_) => tracing::info!("Directory created: {:?}", path),
            Err(e) => {
                eprintln!("Failed to create directory: {}", e);
                return Err(OreMinerError::Io(e));
            }
        }
    }

    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    dotenv().ok();
    ensure_dir_exists(DAEMON_FILE_PATH)?;

    let args = Args::parse();

    setup_logging()?;
    // alert the user based on stderr and then exit the program?
    ensure_binary_exists(&args.ore_binary_path)?;

    let pid_file_path = format!("{}/process.pid", DAEMON_FILE_PATH);
    handle_existing_daemon(&pid_file_path)?;

    start_daemon()?;

    let runtime = setup_runtime()?;
    runtime.block_on(async_main(args))
    // runtime.block_on(async_main_test());
}

#[allow(dead_code)]
async fn async_main_test() {
    let mut count = 0;
    loop {
        tracing::info!("Count: {}", count);
        sleep(Duration::from_secs(1)).await;
        count += 1;
    }
}

fn setup_logging() -> Result<()> {
    let file_appender = RollingFileAppender::new(Rotation::DAILY, "/var/log", "ore-miner.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    fmt()
        .with_writer(std::io::stdout)
        .with_writer(non_blocking.with_min_level(Level::INFO))
        .init();

    Ok(())
}

fn ensure_binary_exists(binary_path: &str) -> Result<()> {
    if binary_path == STANDALONE_BINARY_NAME {
        tracing::info!(
            "No custom binary provided, checking that the default binary = {} is in the system's path",
            binary_path
        );

        match which(binary_path) {
            Ok(_) => tracing::info!("{} is located in the system path", binary_path),
            Err(e) => {
                println!(
                    "Error: The '{}' binary was not found in the system path.",
                    binary_path
                );
                return Err(OreMinerError::BinaryNotFound(
                    binary_path.to_string(),
                    e.to_string(),
                ));
            }
        }
    } else {
        let path = std::path::Path::new(binary_path);
        if !path.exists() {
            println!(
                "Error: The specified binary '{}' does not exist.",
                binary_path
            );
            return Err(OreMinerError::BinaryNotFound(
                binary_path.to_string(),
                "The specified path does not exist".to_string(),
            ));
        }
    }

    Ok(())
}

fn handle_existing_daemon(pid_file_path: &str) -> Result<()> {
    let mut file = File::open(pid_file_path).map_err(|e| OreMinerError::Io(e))?;
    let mut pid = String::new();
    file.read_to_string(&mut pid)
        .map_err(|e| OreMinerError::Io(e))?;

    let pid: i32 = pid.trim().parse().map_err(|e| OreMinerError::PidParse(e))?;
    if is_process_running(pid) {
        tracing::info!("Daemon is already running. Stopping it first.");
        stop_daemon(pid_file_path)
            .map_err(|e| OreMinerError::Daemon(format!("Unable to stop existing daemon: {}", e)))?;
    } else {
        tracing::info!("Removing stale PID file.");
        remove_file(pid_file_path).map_err(|e| OreMinerError::Io(e))?;
    }

    Ok(())
}

fn is_process_running(pid: i32) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn stop_daemon(pid_file_path: &str) -> Result<()> {
    let mut file = File::open(pid_file_path)?;
    let mut pid = String::new();
    file.read_to_string(&mut pid)?;
    let pid: i32 = pid.trim().parse()?;

    std::process::Command::new("kill")
        .arg(pid.to_string())
        .status()
        .map_err(|e| OreMinerError::Daemon(format!("Failed to stop daemon: {}", e)))?;

    tracing::info!("Daemon stopped successfully");

    std::fs::remove_file(pid_file_path)?;

    Ok(())
}

fn start_daemon() -> Result<()> {
    let stdout = File::create(format!("{}/daemon.out", DAEMON_FILE_PATH))
        .map_err(|e| OreMinerError::Io(e))?;
    let stderr = File::create(format!("{}/daemon.err", DAEMON_FILE_PATH))
        .map_err(|e| OreMinerError::Io(e))?;

    println!("Starting daemon... Check logs for subsequent output.");

    /*
     * note: unwrap_or_else has a closure and is more efficient than unwrap_or because it only creates the string if needed
     */
    let user = env::var("USER").unwrap_or_else(|_| "root".to_string());
    let pid_file_path = format!("{}/process.pid", DAEMON_FILE_PATH);

    let daemonize = Daemonize::new()
        .pid_file(&pid_file_path)
        .chown_pid_file(true)
        .working_directory("/tmp")
        .user(user.as_str())
        .group("daemon")
        .umask(0o777)
        .stdout(stdout)
        .stderr(stderr)
        .privileged_action(|| "Daemon privileged action");

    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");

    daemonize.start().map_err(|e| {
        let error_msg = format!("[{:?}] Error starting daemon: {}", timestamp, e);
        tracing::error!("{}", error_msg);
        OreMinerError::Daemon(error_msg)
    })?;

    let success_msg = format!("[{:?}] Daemon started", timestamp);
    tracing::info!("{}", success_msg);

    Ok(())
}

fn setup_runtime() -> Result<Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| OreMinerError::Io(e.into()))
}

async fn async_main(args: Args) -> Result<()> {
    let client = cloudwatch::create_cloudwatch_client().await?;
    let mut command = build_command(&args);

    let mut child = spawn_child_process(&mut command)?;

    let (stdout_handle, stderr_handle) = spawn_output_handlers(&mut child, &client)?;

    let status = child
        .wait()
        .map_err(|e| OreMinerError::CommandExecution(e.to_string()))?;
    tracing::info!("CLI tool exited with status: {}", status);

    stdout_handle.join().expect("Failed to join stdout thread");
    stderr_handle.join().expect("Failed to join stderr thread");

    Ok(())
}

fn build_command(args: &Args) -> Command {
    let mut command = Command::new(&args.ore_binary_path);
    command
        .arg("mine")
        .arg("--cores")
        .arg(&args.cores)
        .arg("--keypair")
        .arg(&args.keypair)
        .arg("--rpc")
        .arg(&args.rpc);

    if !args.fee_payer.is_empty() {
        command.arg("--fee-payer").arg(&args.fee_payer);
    }

    if args.dynamic_fee {
        command.arg("--dynamic-fee");
    }

    if !args.dynamic_fee_url.is_empty() {
        command.arg("--dynamic-fee-url").arg(&args.dynamic_fee_url);
    }

    command
}

fn spawn_child_process(command: &mut Command) -> Result<Child> {
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| OreMinerError::CommandExecution(e.to_string()))
}

fn spawn_output_handlers(
    child: &mut Child,
    client: &Client,
) -> Result<(JoinHandle<()>, JoinHandle<()>)> {
    let stdout = child.stdout.take().ok_or(OreMinerError::CommandExecution(
        "Failed to capture stdout".to_string(),
    ))?;
    let stderr = child.stderr.take().ok_or(OreMinerError::CommandExecution(
        "Failed to capture stderr".to_string(),
    ))?;

    let cloudwatch_client = client.clone();
    let stdout_handle = std::thread::spawn(move || {
        let rt: Runtime = Runtime::new().unwrap();
        let reader = BufReader::new(stdout);
        for line in reader.lines().flatten() {
            rt.block_on(process_output(&line, &cloudwatch_client));
        }
    });

    let stderr_handle = std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().flatten() {
            tracing::error!("CLI tool stderr: {:?}", line);
        }
    });

    Ok((stdout_handle, stderr_handle))
}

async fn process_output(line: &str, client: &Client) {
    tracing::info!("processing line: {}", line);
    match cloudwatch::process_mining_metrics(client, line).await {
        Ok(_) => tracing::info!("Successfully sent metrics to CloudWatch"),
        Err(e) => tracing::error!("Error: {:?}", e),
    }
}
