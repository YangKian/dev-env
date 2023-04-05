use clap::Parser;
use serde_json;
use std::io::{BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::thread;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use dev_env::body::Project;

#[derive(Parser, Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    #[clap(long = "host", default_value = "127.0.0.1")]
    host: String,
    #[clap(long = "port", default_value = "3001")]
    port: usize,
    #[clap(long = "loglevel", default_value = "INFO")]
    log_level: String,
}

fn main() -> Result<(), std::io::Error> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "lsp-server".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = ServerConfig::parse();
    println!("server config: {cli:?}");

    let listener = TcpListener::bind(&format!("{}:{}", cli.host, cli.port))?;

    loop {
        let (mut socket, _) = listener.accept().unwrap();

        let mut buf = vec![0; 1024];
        let n = socket.read(&mut buf).unwrap();
        if n == 0 {
            println!("read 0 bytes");
            continue;
        }

        match serde_json::from_slice::<Project>(&buf[..n]) {
            Ok(project) => {
                thread::spawn(move || {
                    if let Err(e) = run_project(socket, project) {
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

fn run_project(mut stream: TcpStream, project: Project) -> Result<(), std::io::Error> {
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

    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let mut stdin = child.stdin.take().unwrap();

    let mut reader = stream.try_clone().unwrap();
    let mut writer = stream.try_clone().unwrap();

    let t3 = thread::spawn(move || -> Result<(), std::io::Error> {
        let mut buf = vec![0; 4 * 1024];
        let mut r = BufReader::new(stderr);
        loop {
            let n = r.read(&mut buf)?;
            if n != 0 {
                let msg = String::from_utf8_lossy(&buf[..n]);
                eprintln!("get {n} bytes from stderr: {msg}");
            }
        }
    });

    let t2 = thread::spawn(move || -> Result<(), std::io::Error> {
        let mut buf = vec![0; 4 * 1024];
        let mut r = BufReader::new(stdout);
        loop {
            let n = r.read(&mut buf)?;
            if n != 0 {
                // let s = String::from_utf8_lossy(&buf[..n]);
                // println!("read {n} bytes from lsp: {s}");
                writer.write_all(&buf[..n]).unwrap();
                writer.flush().unwrap();
            }
        }
        println!("t2 finished");
        Ok(())
    });

    let t1 = thread::spawn(move || -> Result<(), std::io::Error> {
        let mut buf = vec![0; 4 * 1024];
        let mut r = BufReader::new(reader.try_clone().unwrap());
        loop {
            let n = r.read(&mut buf)?;
            if n != 0 {
                // let s = String::from_utf8_lossy(&buf[..n]);
                stdin.write_all(&buf[..n]).unwrap();
                stdin.flush().unwrap();
            }
        }
        println!("t1 finished");
        Ok(())
    });

    match child.wait() {
        Ok(status) => {
            println!("child exited with status: {status}");
        }
        Err(e) => {
            eprintln!("failed to wait on child, error: {e:?}");
        }
    }

    t1.join().unwrap();
    t2.join().unwrap();
    t3.join().unwrap();
    Ok(())
    // thread::join!(t1, t2, t3);
}
