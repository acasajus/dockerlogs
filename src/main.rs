use colored::*;
use futures::StreamExt;

async fn get_docker() -> docker_api::Docker {
    docker_api::Docker::new("unix:///var/run/docker.sock").unwrap()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let docker = get_docker().await;

    let mut containers =
        futures::future::join_all(
            docker
                .containers()
                .list(&Default::default())
                .await?
                .into_iter()
                .map(|container_info| {
                    let container_id = container_info.id;
                    async move {
                        docker_api::container::Container::new(get_docker().await, container_id)
                    }
                }),
        )
        .await;
    for container in &containers {
        println!(
            "Container {:?}: {:?}",
            container.id(),
            container.top(None).await.unwrap()
        );
    }
    let output_streams = containers
        .drain(0..)
        .enumerate()
        .map(|(index, container)| async move {
            let info = container.inspect().await.unwrap();

            let log_opts = docker_api::api::common::LogsOpts::builder()
                .follow(true)
                .n_lines(10)
                .stdout(true)
                .stderr(true)
                .build();
            let mut stream = container.logs(&log_opts);
            while let Some(data) = stream.next().await {
                let name = &info.name[1..].to_owned();
                let colored_name = match index % 9 {
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
                    Err(err) => eprintln!("{} Error: {:?}", &info.name, err),
                }
            }
        });
    futures::future::join_all(output_streams).await;
    Ok(())
}
