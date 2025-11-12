use clap::{Parser, Subcommand};
use colored::*;
use futures::StreamExt;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;

mod tui;

async fn get_docker(url: &str) -> docker_api::Docker {
    docker_api::Docker::new(url).unwrap()
}

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Docker socket URL
    #[clap(default_value_t = String::from("unix:///var/run/docker.sock"), short, long, value_parser, global = true)]
    url: String,
    /// Containers filter regex
    #[clap(default_value = ".*", short, long, value_parser, global = true)]
    container_regex: String,

    /// Follow docker logs (only for default logs mode)
    #[clap(default_value_t = false, short, long, value_parser)]
    follow: bool,
    /// Show last n lines (only for default logs mode)
    #[clap(default_value_t = 20, short = 'l', long, value_parser)]
    last_n_lines: usize,
    /// Hide stdout (only for default logs mode)
    #[clap(default_value_t = false, short = 'o', long, value_parser)]
    no_stdout: bool,
    /// Hide stderr (only for default logs mode)
    #[clap(default_value_t = false, short = 'e', long, value_parser)]
    no_stderr: bool,

    #[clap(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Interactive TUI mode
    Tui {
        /// Show last n lines
        #[clap(default_value_t = 100, short, long, value_parser)]
        last_n_lines: usize,
    },
}

async fn start_logging_container(
    docker_url: String,
    container_id: String,
    container_regex: regex::Regex,
    log_opts: docker_api::opts::LogsOpts,
    color_index: usize,
    watched_containers: Arc<Mutex<HashSet<String>>>,
    follow: bool,
) {
    let docker = get_docker(&docker_url).await;
    let container = docker_api::container::Container::new(docker, container_id.clone());

    let info = match container.inspect().await {
        Ok(info) => info,
        Err(_) => {
            watched_containers.lock().await.remove(&container_id);
            return;
        }
    };

    let name = match &info.name {
        Some(n) => {
            if n.starts_with('/') {
                n[1..].to_owned()
            } else {
                n.clone()
            }
        }
        None => {
            watched_containers.lock().await.remove(&container_id);
            return;
        }
    };
    if container_regex.find(&name).is_none() {
        watched_containers.lock().await.remove(&container_id);
        return;
    }

    println!(">>> {} Started watching container {}", "✓".bright_green(), name.bright_cyan());

    let mut stream = container.logs(&log_opts);
    while let Some(data) = stream.next().await {
        let colored_name = match color_index % 9 {
            0 => name.bright_green().clone(),
            1 => name.bright_blue(),
            2 => name.bright_yellow(),
            3 => name.bright_magenta(),
            4 => name.bright_cyan(),
            5 => name.bright_white(),
            6 => name.bright_red(),
            7 => name.yellow(),
            8 => name.green(),
            _ => name.on_black().white(),
        };
        match data {
            Ok(contents) => {
                let (descriptor, line) = match contents {
                    docker_api::conn::TtyChunk::StdIn(inner) => {
                        ("i", String::from_utf8_lossy(&inner).into_owned())
                    }
                    docker_api::conn::TtyChunk::StdOut(inner) => {
                        ("o", String::from_utf8_lossy(&inner).into_owned())
                    }
                    docker_api::conn::TtyChunk::StdErr(inner) => {
                        ("e", String::from_utf8_lossy(&inner).into_owned())
                    }
                };
                println!("{} {}: {}", &colored_name, &descriptor, &line.trim())
            }
            Err(_) => {
                break;
            }
        }
    }

    // Container stopped or died, remove from watched list
    if follow {
        println!(">>> {} Container {} stopped", "✗".bright_red(), name.bright_cyan());
    }
    watched_containers.lock().await.remove(&container_id);
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Args::parse();

    match cli.command {
        Some(Command::Tui { last_n_lines }) => {
            tui::run_tui(&cli.url, &cli.container_regex, last_n_lines).await?;
        }
        None => {
            // Default behavior: logs mode
            run_logs_mode(
                &cli.url,
                &cli.container_regex,
                cli.follow,
                cli.last_n_lines,
                cli.no_stdout,
                cli.no_stderr,
            )
            .await?;
        }
    }

    Ok(())
}

async fn run_logs_mode(
    url: &str,
    container_regex_str: &str,
    follow: bool,
    last_n_lines: usize,
    no_stdout: bool,
    no_stderr: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let docker = get_docker(url).await;
    let container_regex = regex::Regex::new(container_regex_str)?;

    // Shared state for tracking watched containers
    let watched_containers: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let color_counter = Arc::new(Mutex::new(0usize));

    let log_opts = docker_api::opts::LogsOpts::builder()
        .follow(follow)
        .n_lines(last_n_lines)
        .stdout(!no_stdout)
        .stderr(!no_stderr)
        .timestamps(false)
        .build();

    // Start logging existing containers
    let containers = docker
        .containers()
        .list(&Default::default())
        .await?;

    let mut tasks = Vec::new();

    for container_info in containers {
        let container_id = match &container_info.id {
            Some(id) => id.clone(),
            None => continue,
        };

        // Check if already watching
        let mut watched = watched_containers.lock().await;
        if watched.contains(&container_id) {
            continue;
        }
        watched.insert(container_id.clone());
        drop(watched);

        let docker_url = url.to_string();
        let regex = container_regex.clone();
        let opts = log_opts.clone();
        let watched = watched_containers.clone();
        let counter = color_counter.clone();

        let is_follow = follow;
        let task = tokio::spawn(async move {
            let mut color_idx = counter.lock().await;
            let idx = *color_idx;
            *color_idx += 1;
            drop(color_idx);

            start_logging_container(
                docker_url,
                container_id,
                regex,
                opts,
                idx,
                watched,
                is_follow,
            )
            .await;
        });
        tasks.push(task);
    }

    // If not following, wait for all tasks to complete and exit
    if !follow {
        for task in tasks {
            let _ = task.await;
        }
        return Ok(());
    }

    // If following, monitor Docker events for new containers
    let event_docker = get_docker(url).await;
    let event_opts = docker_api::opts::EventsOpts::builder().build();

    let mut events = event_docker.events(&event_opts);

    while let Some(event_result) = events.next().await {
        match event_result {
            Ok(event) => {
                // Check if it's a container start event
                if event.type_.as_deref() == Some("container")
                    && event.action.as_deref() == Some("start") {
                    let container_id = match event.actor.and_then(|a| a.id) {
                        Some(id) => id,
                        None => continue,
                    };

                    // Check if already watching
                    let mut watched = watched_containers.lock().await;
                    if watched.contains(&container_id) {
                        continue;
                    }
                    watched.insert(container_id.clone());
                    drop(watched);

                    let docker_url = url.to_string();
                    let regex = container_regex.clone();
                    let opts = log_opts.clone();
                    let watched = watched_containers.clone();
                    let counter = color_counter.clone();

                    tokio::spawn(async move {
                        let mut color_idx = counter.lock().await;
                        let idx = *color_idx;
                        *color_idx += 1;
                        drop(color_idx);

                        start_logging_container(
                            docker_url,
                            container_id,
                            regex,
                            opts,
                            idx,
                            watched,
                            true, // Always true in event loop (follow mode)
                        )
                        .await;
                    });
                }
            }
            Err(_) => {
                // Silently ignore event errors
                continue;
            }
        }
    }

    Ok(())
}
