use clap::Parser;
use colored::*;
use futures::StreamExt;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;

async fn get_docker(url: &str) -> docker_api::Docker {
    docker_api::Docker::new(url).unwrap()
}

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Name of the person to greet
    #[clap(default_value_t = String::from("unix:///var/run/docker.sock"), short, long, value_parser)]
    url: String,
    /// Follow docker logs
    #[clap(default_value_t = false, short, long, value_parser)]
    follow: bool,
    /// Show last n lines
    #[clap(default_value_t = 20, short, long, value_parser)]
    last_n_lines: usize,
    /// Hide stdout
    #[clap(default_value_t = false, short = 'o', long, value_parser)]
    no_stdout: bool,
    /// Hide stderr
    #[clap(default_value_t = false, short = 'e', long, value_parser)]
    no_stderr: bool,
    /// Containers filter regex
    #[clap(default_value = ".*", short, long, value_parser)]
    container_regex: String,
}

async fn start_logging_container(
    docker_url: String,
    container_id: String,
    container_regex: regex::Regex,
    log_opts: docker_api::opts::LogsOpts,
    color_index: usize,
    watched_containers: Arc<Mutex<HashSet<String>>>,
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
    println!(">>> {} Container {} stopped", "✗".bright_red(), name.bright_cyan());
    watched_containers.lock().await.remove(&container_id);
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Args::parse();

    let docker = get_docker(&cli.url).await;
    let container_regex = regex::Regex::new(&cli.container_regex)?;

    // Shared state for tracking watched containers
    let watched_containers: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let color_counter = Arc::new(Mutex::new(0usize));

    let log_opts = docker_api::opts::LogsOpts::builder()
        .follow(cli.follow)
        .n_lines(cli.last_n_lines)
        .stdout(!cli.no_stdout)
        .stderr(!cli.no_stderr)
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

        let docker_url = cli.url.clone();
        let regex = container_regex.clone();
        let opts = log_opts.clone();
        let watched = watched_containers.clone();
        let counter = color_counter.clone();

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
            )
            .await;
        });
        tasks.push(task);
    }

    // If not following, wait for all tasks to complete and exit
    if !cli.follow {
        for task in tasks {
            let _ = task.await;
        }
        return Ok(());
    }

    // If following, monitor Docker events for new containers
    let event_docker = get_docker(&cli.url).await;
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

                    let docker_url = cli.url.clone();
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
