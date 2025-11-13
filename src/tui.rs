use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use std::collections::{HashMap, VecDeque};
use std::io;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
struct ContainerInfo {
    id: String,
    name: String,
    selected: bool,
    color_index: usize,
}

struct AppState {
    containers: Vec<ContainerInfo>,
    list_state: ListState,
    logs: VecDeque<String>,
    max_logs: usize,
    container_logs: HashMap<String, VecDeque<String>>,
    color_counter: usize,
    show_info: bool,
    info_text: String,
    select_all_focused: bool,
}

fn get_color(index: usize) -> Color {
    match index % 9 {
        0 => Color::Cyan,
        1 => Color::Magenta,
        2 => Color::Yellow,
        3 => Color::LightMagenta,
        4 => Color::LightCyan,
        5 => Color::LightGreen,
        6 => Color::LightRed,
        7 => Color::LightYellow,
        8 => Color::LightBlue,
        _ => Color::Cyan,
    }
}

fn strip_ansi_codes(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            // ESC character - start of ANSI escape sequence
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                // Skip until we find a letter (the command character)
                while let Some(&next_ch) = chars.peek() {
                    chars.next();
                    if next_ch.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            result.push(ch);
        }
    }
    result
}

impl AppState {
    fn new(max_logs: usize) -> Self {
        let mut state = Self {
            containers: Vec::new(),
            list_state: ListState::default(),
            logs: VecDeque::with_capacity(max_logs),
            max_logs,
            container_logs: HashMap::new(),
            color_counter: 0,
            show_info: false,
            info_text: String::new(),
            select_all_focused: true,
        };
        state.list_state.select(None);
        state
    }

    fn next(&mut self) {
        if self.select_all_focused {
            self.select_all_focused = false;
            if !self.containers.is_empty() {
                self.list_state.select(Some(0));
            }
        } else {
            let i = match self.list_state.selected() {
                Some(i) => {
                    if i >= self.containers.len() - 1 {
                        self.select_all_focused = true;
                        self.list_state.select(None);
                        return;
                    } else {
                        i + 1
                    }
                }
                None => {
                    self.select_all_focused = true;
                    return;
                }
            };
            self.list_state.select(Some(i));
        }
    }

    fn previous(&mut self) {
        if self.select_all_focused {
            if !self.containers.is_empty() {
                self.select_all_focused = false;
                self.list_state.select(Some(self.containers.len() - 1));
            }
        } else {
            let i = match self.list_state.selected() {
                Some(i) => {
                    if i == 0 {
                        self.select_all_focused = true;
                        self.list_state.select(None);
                        return;
                    } else {
                        i - 1
                    }
                }
                None => {
                    self.select_all_focused = true;
                    return;
                }
            };
            self.list_state.select(Some(i));
        }
    }

    fn toggle_selected(&mut self) {
        if self.select_all_focused {
            // Toggle all containers
            let all_selected = self.containers.iter().all(|c| c.selected);
            if all_selected {
                self.deselect_all();
            } else {
                self.select_all();
            }
        } else if let Some(i) = self.list_state.selected() {
            if i < self.containers.len() {
                self.containers[i].selected = !self.containers[i].selected;
                self.update_displayed_logs();
            }
        }
    }

    fn select_all(&mut self) {
        for container in &mut self.containers {
            container.selected = true;
        }
        self.update_displayed_logs();
    }

    fn deselect_all(&mut self) {
        for container in &mut self.containers {
            container.selected = false;
        }
        self.update_displayed_logs();
    }

    fn selected_count(&self) -> usize {
        self.containers.iter().filter(|c| c.selected).count()
    }

    fn add_log(&mut self, container_name: &str, log_line: String) {
        // Add to container-specific logs
        let container_logs = self
            .container_logs
            .entry(container_name.to_string())
            .or_insert_with(|| VecDeque::with_capacity(self.max_logs));

        container_logs.push_back(log_line.clone());
        if container_logs.len() > self.max_logs {
            container_logs.pop_front();
        }

        // Update displayed logs if this container is selected
        if self.is_container_selected(container_name) {
            self.logs.push_back(log_line);
            if self.logs.len() > self.max_logs {
                self.logs.pop_front();
            }
        }
    }

    fn is_container_selected(&self, container_name: &str) -> bool {
        self.containers
            .iter()
            .any(|c| c.name == container_name && c.selected)
    }

    fn update_displayed_logs(&mut self) {
        self.logs.clear();
        let selected_containers: Vec<String> = self
            .containers
            .iter()
            .filter(|c| c.selected)
            .map(|c| c.name.clone())
            .collect();

        // Merge logs from all selected containers
        let mut all_logs: Vec<String> = Vec::new();
        for container_name in &selected_containers {
            if let Some(container_logs) = self.container_logs.get(container_name) {
                all_logs.extend(container_logs.iter().cloned());
            }
        }

        // Take the last max_logs entries
        let start = if all_logs.len() > self.max_logs {
            all_logs.len() - self.max_logs
        } else {
            0
        };
        self.logs.extend(all_logs[start..].iter().cloned());
    }

    fn add_container(&mut self, id: String, name: String) {
        if !self.containers.iter().any(|c| c.id == id) {
            let color_index = self.color_counter;
            self.color_counter += 1;
            self.containers.push(ContainerInfo {
                id,
                name,
                selected: true, // Auto-select new containers
                color_index,
            });
            self.containers.sort_by(|a, b| a.name.cmp(&b.name));
            // If this is the first container, select it
            if self.containers.len() == 1 {
                self.list_state.select(Some(0));
            }
            self.update_displayed_logs();
        }
    }

    fn get_container_color(&self, container_name: &str) -> Option<Color> {
        self.containers
            .iter()
            .find(|c| c.name == container_name)
            .map(|c| get_color(c.color_index))
    }

    fn remove_container(&mut self, id: &str) {
        let removed_name = self
            .containers
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.name.clone());

        self.containers.retain(|c| c.id != id);

        // Smart auto-selection logic
        if let Some(name) = removed_name {
            // Try to select a container with the same name
            let same_name_exists = self.containers.iter().any(|c| c.name == name);
            if same_name_exists {
                // Select all containers with the same name
                for container in &mut self.containers {
                    if container.name == name {
                        container.selected = true;
                    }
                }
            } else if !self.containers.is_empty() {
                // No container with the same name, select all
                self.select_all();
            }

            // Clean up logs for removed container
            self.container_logs.remove(&name);
        }

        // Adjust selection if needed
        if self.containers.is_empty() {
            self.list_state.select(None);
        } else if let Some(i) = self.list_state.selected() {
            if i >= self.containers.len() {
                self.list_state.select(Some(self.containers.len() - 1));
            }
        }

        self.update_displayed_logs();
    }

    fn max_container_name_width(&self) -> u16 {
        self.containers
            .iter()
            .map(|c| c.name.len())
            .max()
            .unwrap_or(20)
            .max(20) as u16
            + 3
    }
}

fn ui(f: &mut Frame, app: &mut AppState) {
    let size = f.area();

    // Calculate left pane width based on longest container name
    let left_width = app.max_container_name_width().min(size.width / 3);

    // Main layout with help line at bottom
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(size);

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(left_width),
            Constraint::Min(1), // Ensure right pane has space
        ])
        .margin(0)
        .split(main_chunks[0]);

    // Split left pane into "All" and container list
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(chunks[0]);

    // "All" selector
    let all_selected = app.containers.iter().all(|c| c.selected);
    let checkbox = if all_selected { "◉" } else { "○" };
    let checkbox_color = if all_selected {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let select_all_line = if app.select_all_focused {
        Line::from(vec![
            Span::styled(
                "▶ ",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{} ", checkbox),
                Style::default().fg(Color::Black).bg(Color::Magenta),
            ),
            Span::styled(
                "ALL",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                format!("{} ", checkbox),
                Style::default().fg(checkbox_color),
            ),
            Span::styled(
                "ALL",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ])
    };

    // Right pane: logs or info (render FIRST to prevent overflow)
    if app.show_info {
        let info_paragraph = Paragraph::new(app.info_text.as_str())
            .style(Style::default().fg(Color::Cyan))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )
                    .title("▶ CONTAINER INFO")
                    .title_style(
                        Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .wrap(Wrap { trim: false })
            .alignment(Alignment::Left);
        f.render_widget(info_paragraph, chunks[1]);
    } else {
        let selected_count = app.selected_count();
        let show_container_names = selected_count != 1;

        // Calculate available width for text with conservative margin
        // Account for borders (2) + styling overhead + safety buffer
        // Now that we sanitize input, can use a more reasonable margin
        let max_width = if chunks[1].width > 8 {
            (chunks[1].width - 8) as usize
        } else {
            5
        };

        let log_text: Vec<Line> = app
            .logs
            .iter()
            .map(|line| {
                // Sanitize the line - remove control characters and ANSI codes that mess up display
                let without_ansi = strip_ansi_codes(line);
                let sanitized = without_ansi
                    .chars()
                    .filter(|c| !c.is_control() || *c == ' ')
                    .collect::<String>()
                    .replace('\r', "")
                    .replace('\n', " ")
                    .replace('\t', "    ");

                let line = &sanitized;

                // First, build the full line
                let full_line = if show_container_names {
                    // Parse log line format: "container_name descriptor: log_text"
                    if let Some(first_space_idx) = line.find(' ') {
                        let container_name = &line[..first_space_idx];
                        let rest = &line[first_space_idx..];

                        if let Some(color) = app.get_container_color(container_name) {
                            (Some(container_name.to_string()), Some(color), rest.to_string())
                        } else {
                            (None, None, line.clone())
                        }
                    } else {
                        (None, None, line.clone())
                    }
                } else {
                    // Only one container selected, skip container name
                    // Format: "container_name descriptor: log_text" -> "descriptor: log_text"
                    if let Some(first_space_idx) = line.find(' ') {
                        (None, None, line[first_space_idx + 1..].to_string())
                    } else {
                        (None, None, line.clone())
                    }
                };

                // Simply truncate each line to max_width - no wrapping
                let (container_name, color, rest) = full_line;

                if let (Some(name), Some(c)) = (container_name, color) {
                    let prefix_width = name.width();
                    let remaining_width = max_width.saturating_sub(prefix_width).saturating_sub(2);

                    // Truncate text to fit within remaining width
                    let mut truncated = String::new();
                    let mut current_width = 0;
                    for ch in rest.chars() {
                        let ch_width = ch.width().unwrap_or(0);
                        if current_width + ch_width >= remaining_width {
                            break;
                        }
                        truncated.push(ch);
                        current_width += ch_width;
                    }

                    Line::from(vec![
                        Span::styled(name, Style::default().fg(c).add_modifier(Modifier::BOLD)),
                        Span::raw(truncated),
                    ])
                } else {
                    // No container name, just truncate the text
                    let mut truncated = String::new();
                    let mut current_width = 0;
                    for ch in rest.chars() {
                        let ch_width = ch.width().unwrap_or(0);
                        if current_width + ch_width >= max_width {
                            break;
                        }
                        truncated.push(ch);
                        current_width += ch_width;
                    }
                    Line::from(truncated)
                }
            })
            .collect();

        // Final safety check: ensure no line exceeds max width
        // Use the SAME conservative width as truncation to ensure consistency
        let max_line_width = max_width;
        let log_text: Vec<Line> = log_text
            .into_iter()
            .map(|line| {
                // Calculate the actual display width of the line
                let line_width: usize = line.spans.iter().map(|span| span.content.width()).sum();

                if line_width >= max_line_width {
                    // Truncate the line if it's too long
                    let mut new_spans = Vec::new();
                    let mut current_width = 0;

                    for span in line.spans {
                        let span_width = span.content.width();
                        if current_width + span_width < max_line_width {
                            new_spans.push(span);
                            current_width += span_width;
                        } else {
                            // Truncate this span
                            let remaining = max_line_width.saturating_sub(current_width);
                            if remaining > 0 {
                                let mut truncated = String::new();
                                let mut w = 0;
                                for ch in span.content.chars() {
                                    let ch_w = ch.width().unwrap_or(0);
                                    if w + ch_w >= remaining {
                                        break;
                                    }
                                    truncated.push(ch);
                                    w += ch_w;
                                }
                                new_spans.push(Span::styled(truncated, span.style));
                            }
                            break;
                        }
                    }
                    Line::from(new_spans)
                } else {
                    line
                }
            })
            .collect();

        // Calculate scroll to show latest logs at bottom
        let block_height = chunks[1].height.saturating_sub(2); // Account for borders
        let log_count = log_text.len();
        let scroll_offset = if log_count > block_height as usize {
            (log_count - block_height as usize) as u16
        } else {
            0
        };

        let paragraph = Paragraph::new(log_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )
                    .title("▶ LOGS")
                    .title_style(
                        Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .alignment(Alignment::Left)
            .scroll((scroll_offset, 0));

        f.render_widget(paragraph, chunks[1]);
    }

    // Left pane: render AFTER right pane to ensure it's on top
    let select_all_widget = Paragraph::new(select_all_line).block(
        Block::default().borders(Borders::ALL).border_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    );
    f.render_widget(select_all_widget, left_chunks[0]);

    // Container list
    let items: Vec<ListItem> = app
        .containers
        .iter()
        .map(|c| {
            let checkbox = if c.selected { "◉" } else { "○" };
            let checkbox_style = if c.selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let color = get_color(c.color_index);
            let line = Line::from(vec![
                Span::styled(format!("{} ", checkbox), checkbox_style),
                Span::styled(
                    &c.name,
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )
                .title("▶ CONTAINERS")
                .title_style(
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Magenta)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    f.render_stateful_widget(list, left_chunks[1], &mut app.list_state);

    // Help line at bottom
    let help_text = if app.show_info {
        "↑/↓: Navigate | Enter/Space: Toggle | i: Close Info | a: All | n: None | Esc/q: Quit"
    } else {
        "↑/↓: Navigate | Enter/Space: Toggle | i: Show Info | a: All | n: None | Esc/q: Quit"
    };

    let help_spans = vec![
        Span::styled(
            "◆ ",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(help_text, Style::default().fg(Color::Cyan)),
    ];

    let help_widget = Paragraph::new(Line::from(help_spans)).block(
        Block::default().borders(Borders::ALL).border_style(
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
    );
    f.render_widget(help_widget, main_chunks[1]);
}

async fn get_container_info(docker_url: &str, container_id: &str) -> String {
    let docker = crate::get_docker(docker_url).await;
    let container = docker_api::container::Container::new(docker, container_id.to_string());

    match container.inspect().await {
        Ok(info) => {
            let mut output = String::new();
            output.push_str(&format!("ID: {}\n", container_id));
            if let Some(name) = &info.name {
                output.push_str(&format!("Name: {}\n", name));
            }
            if let Some(state) = &info.state {
                output.push_str(&format!("Status: {:?}\n", state.status));
                if let Some(running) = state.running {
                    output.push_str(&format!("Running: {}\n", running));
                }
                if let Some(started_at) = &state.started_at {
                    output.push_str(&format!("Started: {}\n", started_at));
                }
            }
            if let Some(config) = &info.config {
                if let Some(image) = &config.image {
                    output.push_str(&format!("Image: {}\n", image));
                }
                if let Some(hostname) = &config.hostname {
                    output.push_str(&format!("Hostname: {}\n", hostname));
                }
            }
            if let Some(network_settings) = &info.network_settings {
                if let Some(networks) = &network_settings.networks {
                    output.push_str("\nNetworks:\n");
                    for (name, network) in networks {
                        output.push_str(&format!("  {}: ", name));
                        if let Some(ip) = &network.ip_address {
                            output.push_str(&ip.to_string());
                        }
                        output.push('\n');
                    }
                }
            }
            output
        }
        Err(e) => format!("Failed to inspect container: {:?}", e),
    }
}

async fn log_container(
    docker_url: String,
    container_id: String,
    container_regex: regex::Regex,
    last_n_lines: usize,
    app_state: Arc<Mutex<AppState>>,
) {
    let docker = crate::get_docker(&docker_url).await;
    let container = docker_api::container::Container::new(docker, container_id.clone());

    let info = match container.inspect().await {
        Ok(info) => info,
        Err(_) => return,
    };

    let name = match &info.name {
        Some(n) => {
            if let Some(stripped) = n.strip_prefix('/') {
                stripped.to_owned()
            } else {
                n.clone()
            }
        }
        None => return,
    };

    if container_regex.find(&name).is_none() {
        return;
    }

    // Add container to the list
    {
        let mut app = app_state.lock().await;
        app.add_container(container_id.clone(), name.clone());
    }

    let log_opts = docker_api::opts::LogsOpts::builder()
        .follow(true)
        .n_lines(last_n_lines)
        .stdout(true)
        .stderr(true)
        .timestamps(false)
        .build();

    let mut stream = container.logs(&log_opts);
    while let Some(data) = stream.next().await {
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
                let log_line = format!("{} {}: {}", name, descriptor, line.trim());
                let mut app = app_state.lock().await;
                app.add_log(&name, log_line);
            }
            Err(_) => break,
        }
    }

    // Container stopped
    {
        let mut app = app_state.lock().await;
        app.remove_container(&container_id);
    }
}

pub async fn run_tui(
    url: &str,
    container_regex_str: &str,
    last_n_lines: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let app_state = Arc::new(Mutex::new(AppState::new(last_n_lines * 10)));
    let docker = crate::get_docker(url).await;
    let container_regex = regex::Regex::new(container_regex_str)?;

    // Spawn container log tasks
    let containers = docker.containers().list(&Default::default()).await?;
    for container_info in containers {
        let container_id = match &container_info.id {
            Some(id) => id.clone(),
            None => continue,
        };

        let docker_url = url.to_string();
        let regex = container_regex.clone();
        let app = app_state.clone();

        tokio::spawn(async move {
            log_container(docker_url, container_id, regex, last_n_lines, app).await;
        });
    }

    // Spawn event monitoring task
    let event_app_state = app_state.clone();
    let event_url = url.to_string();
    let event_regex = container_regex.clone();
    tokio::spawn(async move {
        let event_docker = crate::get_docker(&event_url).await;
        let event_opts = docker_api::opts::EventsOpts::builder().build();
        let mut events = event_docker.events(&event_opts);

        while let Some(event_result) = events.next().await {
            if let Ok(event) = event_result {
                if event.type_.as_deref() == Some("container")
                    && event.action.as_deref() == Some("start")
                {
                    if let Some(container_id) = event.actor.and_then(|a| a.id) {
                        let docker_url = event_url.clone();
                        let regex = event_regex.clone();
                        let app = event_app_state.clone();

                        tokio::spawn(async move {
                            log_container(docker_url, container_id, regex, last_n_lines, app).await;
                        });
                    }
                }
            }
        }
    });

    // Main UI loop
    let docker_url_clone = url.to_string();
    tokio::task::spawn_blocking(
        move || -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            loop {
                // Render UI
                {
                    let mut app = tokio::runtime::Handle::current().block_on(app_state.lock());
                    terminal.draw(|f| ui(f, &mut app))?;
                }

                // Handle input
                if event::poll(std::time::Duration::from_millis(100))? {
                    if let Event::Key(key) = event::read()? {
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => break,
                            KeyCode::Down | KeyCode::Char('j') => {
                                let mut app =
                                    tokio::runtime::Handle::current().block_on(app_state.lock());
                                app.next();
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                let mut app =
                                    tokio::runtime::Handle::current().block_on(app_state.lock());
                                app.previous();
                            }
                            KeyCode::Char(' ') | KeyCode::Enter => {
                                let mut app =
                                    tokio::runtime::Handle::current().block_on(app_state.lock());
                                app.toggle_selected();
                            }
                            KeyCode::Char('a') => {
                                let mut app =
                                    tokio::runtime::Handle::current().block_on(app_state.lock());
                                app.select_all();
                            }
                            KeyCode::Char('n') => {
                                let mut app =
                                    tokio::runtime::Handle::current().block_on(app_state.lock());
                                app.deselect_all();
                            }
                            KeyCode::Char('i') => {
                                let mut app =
                                    tokio::runtime::Handle::current().block_on(app_state.lock());

                                if app.show_info {
                                    // Close info panel
                                    app.show_info = false;
                                } else if let Some(selected_idx) = app.list_state.selected() {
                                    // Show info for selected container
                                    if selected_idx < app.containers.len() {
                                        let container_id = app.containers[selected_idx].id.clone();
                                        let docker_url = docker_url_clone.clone();
                                        drop(app); // Release lock before async operation

                                        let info = tokio::runtime::Handle::current().block_on(
                                            get_container_info(&docker_url, &container_id),
                                        );

                                        let mut app = tokio::runtime::Handle::current()
                                            .block_on(app_state.lock());
                                        app.info_text = info;
                                        app.show_info = true;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }

            // Restore terminal
            disable_raw_mode()?;
            execute!(
                terminal.backend_mut(),
                LeaveAlternateScreen,
                DisableMouseCapture
            )?;
            terminal.show_cursor()?;

            Ok(())
        },
    )
    .await
    .map_err(|e| format!("Task join error: {}", e))?
    .map_err(|e| format!("{}", e))?;

    Ok(())
}
