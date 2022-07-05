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
    let output_streams = containers.drain(0..).map(|container| async move {
        let info = container.inspect().await.unwrap();

        let log_opts = docker_api::api::common::LogsOpts::builder()
            .follow(true)
            .n_lines(10)
            .stdout(true)
            .stderr(true)
            .build();
        let mut stream = container.logs(&log_opts);
        while let Some(data) = stream.next().await {
            match data {
                Ok(contents) => {
                    let (descriptor, line) = match contents {
                        docker_api::conn::TtyChunk::StdIn(inner) => {
                            (0, String::from_utf8_lossy(&inner).into_owned())
                        }
                        docker_api::conn::TtyChunk::StdOut(inner) => {
                            (1, String::from_utf8_lossy(&inner).into_owned())
                        }
                        docker_api::conn::TtyChunk::StdErr(inner) => {
                            (2, String::from_utf8_lossy(&inner).into_owned())
                        }
                    };
                    println!("{} [{}]: {}", &info.name, &descriptor, &line.trim())
                }
                Err(err) => eprintln!("{} Error: {:?}", &info.name, err),
            }
        }
    });
    futures::future::join_all(output_streams).await;
    Ok(())
}
