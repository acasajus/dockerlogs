use clap::Parser;
use colored::*;
use futures::StreamExt;

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Args::parse();

    let docker = get_docker(&cli.url).await;

    let container_regex = regex::Regex::new(&cli.container_regex)?;

    let mut containers =
        futures::future::join_all(
            docker
                .containers()
                .list(&Default::default())
                .await?
                .into_iter()
                .map(|container_info| {
                    let container_id = container_info.id;
                    let url = &cli.url;
                    async move {
                        docker_api::container::Container::new(get_docker(url).await, container_id)
                    }
                }),
        )
        .await;
    let output_streams = containers.drain(0..).enumerate().map(|(index, container)| {
        let log_opts = docker_api::api::common::LogsOpts::builder()
            .follow(cli.follow)
            .n_lines(cli.last_n_lines)
            .stdout(!cli.no_stdout)
            .stderr(!cli.no_stderr)
            .build();

        let name_regex = container_regex.clone();

        async move {
            let info = container.inspect().await.unwrap();
            let name = &info.name[1..].to_owned();
            if name_regex.find(&name).is_none() {
                return;
            }

            let mut stream = container.logs(&log_opts);
            while let Some(data) = stream.next().await {
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
        }
    });
    futures::future::join_all(output_streams).await;
    Ok(())
}
