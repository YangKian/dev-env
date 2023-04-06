use clap::Parser;
use serde_json;
use std::process::Stdio;
use tokio::io::{copy, AsyncBufReadExt, AsyncReadExt, BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Command;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use dev_env::proto::Project;

#[derive(Parser, Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    #[clap(long = "host", default_value = "127.0.0.1")]
    host: String,
    #[clap(long = "port", default_value = "3001")]
    port: usize,
    #[clap(long = "loglevel", default_value = "INFO")]
    log_level: String,
}

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "lsp-server".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = ServerConfig::parse();
    println!("server config: {cli:?}");

    let listener = TcpListener::bind(&format!("{}:{}", cli.host, cli.port)).await?;

    loop {
        let (mut socket, _) = listener.accept().await.unwrap();

        let mut buf = vec![0; 1024];
        let n = socket.read(&mut buf).await.unwrap();
        if n == 0 {
            println!("read 0 bytes");
            continue;
        }

        match serde_json::from_slice::<Project>(&buf[..n]) {
            Ok(project) => {
                tokio::spawn(async move {
                    if let Err(e) = run_project(socket, project).await {
                        eprintln!("failed to run project, error: {e:?}");
                        return;
                    }
                    println!("project finished");
                });
            }
            Err(e) => {
                eprintln!("failed to parse project, error: {e:?}");
            }
        }
    }
}

async fn run_project(stream: TcpStream, project: Project) -> Result<(), tokio::io::Error> {
    println!("receive init request, project: {project:?}");
    let mut child = Command::new(project.command)
        .args(project.args)
        .current_dir(project.root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn child");
    println!("spawned child successfully");

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let stdin = child.stdin.take().unwrap();

    let (mut reader, mut writer) = stream.into_split();

    let log_stream = tokio::spawn(async move {
        let mut r = BufReader::new(stderr).lines();
        while let Some(line_result) = r.next_line().await? {
            eprintln!("[lsp info]: {}", line_result);
        }
        println!("log_stream finished");
        Ok::<(), tokio::io::Error>(())
    });

    // forward lsp server response to lsp client
    let output_stream = tokio::spawn(async move {
        let mut reader = BufReader::new(stdout);
        if let Err(e) = copy(&mut reader, &mut writer).await {
            eprintln!("Output stream err: {}", e);
        }
        println!("output_stream finished");
        Ok::<(), tokio::io::Error>(())
    });

    // forward lsp client request to lsp server
    let input_stream = tokio::spawn(async move {
        let mut reader = BufReader::new(reader);
        let mut writer = BufWriter::new(stdin);
        if let Err(e) = copy(&mut reader, &mut writer).await {
            eprintln!("Input stream err: {}", e);
        }
        println!("input_stream finished");
        Ok::<(), tokio::io::Error>(())
    });

    tokio::join!(input_stream, output_stream, log_stream);
    match child.wait().await {
        Ok(status) => {
            println!("child exited with status: {status}");
        }
        Err(e) => {
            eprintln!("failed to wait on child, error: {e:?}");
        }
    }
    Ok::<(), tokio::io::Error>(())
}
