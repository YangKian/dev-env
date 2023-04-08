use clap::Parser;
use serde_yaml;
use std::env::current_dir;
use std::fs::File;
use tokio::io::{
    copy, stdin, stdout, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, BufWriter,
};
use tokio::net::TcpStream;
use tracing_appender;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use dev_env::proto::Project;

#[derive(Parser, Debug, Clone, PartialEq, Eq)]
pub struct ClientConfig {
    #[clap(long = "host", default_value = "127.0.0.1")]
    host: String,
    #[clap(long = "port", default_value = "3001")]
    port: usize,
    #[clap(long = "loglevel", default_value = "INFO")]
    log_level: String,
    #[clap(long = "logpath", default_value = "/tmp/lsp-client.log")]
    log_path: String,
    #[clap(long = "projects", help = "projects config file")]
    projects: String,
}

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    let cli = ClientConfig::parse();

    let log_file = File::create(cli.log_path.clone()).unwrap();
    let (log_writer, _guard) = tracing_appender::non_blocking(std::io::BufWriter::new(log_file));
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(cli.log_level.as_str())),
        )
        .with(tracing_subscriber::fmt::layer().with_writer(log_writer))
        .init();

    tracing::info!("client config: {cli:?}");

    let cwd = current_dir().unwrap().to_string_lossy().to_string();
    let config_file = File::open(cli.projects).unwrap();
    let projects: Vec<Project> = serde_yaml::from_reader(config_file).unwrap();
    let project = find_project(cwd.as_str(), projects.as_slice()).unwrap();
    tracing::info!("target project: {project:?}");

    let connection = TcpStream::connect(&format!("{}:{}", cli.host, cli.port))
        .await
        .unwrap();
    tracing::info!("connected to server successfully");

    let (reader, writer) = connection.into_split();
    let mut buf_stdin = BufReader::new(stdin());
    let mut buf_stdout = BufWriter::new(stdout());
    let mut reader = BufReader::new(reader);
    let mut writer = BufWriter::new(writer);

    // send first request
    let request = serde_json::to_vec(&project).unwrap();
    writer.write_all(request.as_slice()).await?;
    writer.flush().await?;

    // forward client request to server
    let request_stream = tokio::spawn(async move {
        if let Err(e) = copy(&mut buf_stdin, &mut writer).await {
            tracing::error!("failed to write to stream, error: {e:?}");
            return;
        }
        // let mut buffer = vec![0; 10 * 1024];
        // while let Ok(n) = buf_stdin.read(&mut buffer).await {
        //     let msg = String::from_utf8(buffer[..n].to_vec()).unwrap();
        //     tracing::info!("read {n} bytes from stdin: P{msg:?}");
        //     writer.write_all(&buffer[..n]).await.unwrap();
        //     writer.flush().await.unwrap();
        // }
    });

    // forward server response to client
    let response_stream = tokio::spawn(async move {
        if let Err(e) = copy(&mut reader, &mut buf_stdout).await {
            tracing::error!("failed to read from stream, error: {e:?}");
            return;
        }

        // let mut buffer = vec![0; 10 * 1024];
        // while let Ok(n) = reader.read(&mut buffer).await {
        //     let msg = String::from_utf8(buffer[..n].to_vec()).unwrap();
        //     tracing::info!("read {n} bytes from tcp stream: P{msg:?}");
        //     buf_stdout.write_all(&buffer[..n]).await.unwrap();
        //     buf_stdout.flush().await.unwrap();
        // }
    });

    tokio::join!(request_stream, response_stream);
    Ok(())
}

fn find_project(target: &str, projects: &[Project]) -> Option<Project> {
    projects
        .iter()
        .find(|&p| target.starts_with(p.root.as_str()))
        .cloned()
}
