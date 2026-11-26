//! `arena board`: live terminal view of the round.
//!
//! Tabs: time and memory leaderboards, a time-vs-memory scatter, rank over time
//! (by CPU time) and, when logged in, your own submissions.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use anyhow::Result;
use arena_protocol::{
    AcceptedSubmission, Board, BoardEntry, RoundInfo, RoundState, ServerEvent, SubmissionInfo,
    SubmissionStatus, Verdict,
};
use crossterm::event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Axis, Block, Cell, Chart, Dataset, GraphType, Paragraph, Row, Table, Tabs};

use crate::client::Client;
use crate::format;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Time,
    Memory,
    Scatter,
    Ranks,
    Mine,
}

const TABS: [(Tab, &str); 5] = [
    (Tab::Time, "1 Time"),
    (Tab::Memory, "2 Memory"),
    (Tab::Scatter, "3 Time × memory"),
    (Tab::Ranks, "4 Rank over time"),
    (Tab::Mine, "5 My submissions"),
];

/// Distinct colours for chart series, cycled.
const PALETTE: [Color; 10] = [
    Color::Cyan,
    Color::Yellow,
    Color::Magenta,
    Color::Green,
    Color::LightRed,
    Color::LightBlue,
    Color::LightGreen,
    Color::LightMagenta,
    Color::White,
    Color::Red,
];

#[derive(Default)]
pub struct State {
    pub me: Option<String>,
    pub round: Option<RoundInfo>,
    pub boards: HashMap<String, Board>,
    pub history: HashMap<String, Vec<AcceptedSubmission>>,
    pub mine: Vec<SubmissionInfo>,
    pub problem: usize,
    pub tab: Option<Tab>,
    pub status: String,
    /// Server clock minus ours, in seconds, so the countdown matches the server.
    pub clock_offset: i64,
}

impl State {
    fn now(&self) -> i64 {
        format::now() + self.clock_offset
    }

    fn tab(&self) -> Tab {
        self.tab.unwrap_or(Tab::Time)
    }

    fn problems(&self) -> Vec<String> {
        match &self.round {
            Some(r) => r.problems.iter().map(|p| p.slug.clone()).collect(),
            None => {
                let mut v: Vec<_> = self.boards.keys().cloned().collect();
                v.sort();
                v
            }
        }
    }

    fn selected(&self) -> Option<String> {
        let p = self.problems();
        p.get(self.problem.min(p.len().saturating_sub(1))).cloned()
    }

    /// Applies a server event; returns problems whose history should be refetched.
    pub fn apply(&mut self, event: ServerEvent) -> Vec<String> {
        match event {
            ServerEvent::Hello { user } => {
                self.me = user;
                Vec::new()
            }
            ServerEvent::Round { round } => {
                let changed = self.round.as_ref().map(|r| r.id) != Some(round.id);
                if changed {
                    self.boards.clear();
                    self.history.clear();
                    self.problem = 0;
                }
                self.clock_offset = round.server_time - format::now();
                self.round = Some(round);
                Vec::new()
            }
            ServerEvent::Board { board } => {
                let slug = board.problem.clone();
                self.boards.insert(slug.clone(), board);
                vec![slug]
            }
            ServerEvent::Progress { .. } => Vec::new(),
            ServerEvent::Submission { submission } => {
                match self.mine.iter_mut().find(|s| s.id == submission.id) {
                    Some(s) => *s = submission,
                    None => self.mine.push(submission),
                }
                Vec::new()
            }
        }
    }
}

pub async fn run(client: Client, logged_in: bool) -> Result<()> {
    let mut state = State {
        status: format!("connecting to {}", client.base()),
        ..State::default()
    };
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &client, logged_in, &mut state).await;
    ratatui::restore();
    result
}

async fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    client: &Client,
    logged_in: bool,
    state: &mut State,
) -> Result<()> {
    let mut keys = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut events = None;
    let mut retry_at = tokio::time::Instant::now();

    loop {
        if events.is_none() && tokio::time::Instant::now() >= retry_at {
            match client.events().await {
                Ok(stream) => {
                    events = Some(stream);
                    state.status = format!("live — {}", client.base());
                    if logged_in {
                        refresh_mine(client, state).await;
                    }
                }
                Err(e) => {
                    state.status = format!("offline ({e:#}); retrying");
                    retry_at = tokio::time::Instant::now() + Duration::from_secs(3);
                }
            }
        }
        terminal.draw(|f| draw(f, state))?;

        let next_event = async {
            match events.as_mut() {
                Some(s) => s.next().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = tick.tick() => {}
            key = keys.next() => {
                if let Some(Ok(Event::Key(k))) = key
                    && k.kind == KeyEventKind::Press
                    && handle_key(state, k.code, k.modifiers)
                {
                    return Ok(());
                }
            }
            event = next_event => match event {
                Ok(Some(e)) => {
                    let mine_changed = matches!(e, ServerEvent::Submission { .. });
                    for slug in state.apply(e) {
                        refresh_history(client, state, &slug).await;
                    }
                    if mine_changed {
                        refresh_mine(client, state).await;
                    }
                }
                Ok(None) | Err(_) => {
                    events = None;
                    state.status = "connection lost; reconnecting".into();
                    retry_at = tokio::time::Instant::now() + Duration::from_secs(1);
                }
            },
        }
    }
}

async fn refresh_history(client: &Client, state: &mut State, slug: &str) {
    let Some(round) = state.round.as_ref().map(|r| r.id) else {
        return;
    };
    if let Ok(h) = client
        .get::<Vec<AcceptedSubmission>>(&format!("/api/rounds/{round}/problems/{slug}/accepted"))
        .await
    {
        state.history.insert(slug.to_owned(), h);
    }
}

async fn refresh_mine(client: &Client, state: &mut State) {
    if let Ok(list) = client
        .get::<Vec<SubmissionInfo>>("/api/me/submissions")
        .await
    {
        state.mine = list;
    }
}

/// Returns true to quit.
pub fn handle_key(state: &mut State, code: KeyCode, mods: KeyModifiers) -> bool {
    let n = state.problems().len().max(1);
    match code {
        KeyCode::Char('q') | KeyCode::Esc => return true,
        KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => return true,
        KeyCode::Char(c @ '1'..='5') => state.tab = Some(TABS[(c as u8 - b'1') as usize].0),
        KeyCode::Tab => {
            let i = TABS
                .iter()
                .position(|(t, _)| *t == state.tab())
                .unwrap_or(0);
            state.tab = Some(TABS[(i + 1) % TABS.len()].0);
        }
        KeyCode::Right | KeyCode::Char('l') => state.problem = (state.problem + 1) % n,
        KeyCode::Left | KeyCode::Char('h') => state.problem = (state.problem + n - 1) % n,
        _ => {}
    }
    false
}

pub fn draw(f: &mut Frame, state: &State) {
    let [header, tabs, body, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Min(5),
        Constraint::Length(1),
    ])
    .areas(f.area());

    f.render_widget(Paragraph::new(header_lines(state)), header);
    let titles: Vec<&str> = TABS.iter().map(|(_, t)| *t).collect();
    let selected = TABS
        .iter()
        .position(|(t, _)| *t == state.tab())
        .unwrap_or(0);
    f.render_widget(
        Tabs::new(titles)
            .select(selected)
            .highlight_style(Style::new().reversed()),
        tabs,
    );

    let problem = state.selected();
    let board = problem.as_ref().and_then(|p| state.boards.get(p));
    match state.tab() {
        Tab::Time => draw_table(f, body, state, board, true),
        Tab::Memory => draw_table(f, body, state, board, false),
        Tab::Scatter => draw_scatter(f, body, board),
        Tab::Ranks => draw_ranks(f, body, state, problem.as_deref()),
        Tab::Mine => draw_mine(f, body, state),
    }
    let help = "←/→ problem   1-5/Tab view   q quit";
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw(help).dark_gray(),
            Span::raw("   "),
            Span::raw(state.status.as_str()).dark_gray(),
        ])),
        footer,
    );
}

fn header_lines(state: &State) -> Vec<Line<'_>> {
    let Some(round) = &state.round else {
        return vec![Line::from("Waiting for a round to start…".bold())];
    };
    let now = state.now();
    let clock = match round.state {
        RoundState::Draft => "not started".to_owned(),
        RoundState::Running => format!(
            "{} left",
            format::duration(round.ends_at.unwrap_or(now) - now)
        ),
        RoundState::Finished => "finished".to_owned(),
    };
    let frozen = state.boards.values().any(|b| b.frozen);
    let mut first = vec![
        Span::raw(format!("{}  ", round.name)).bold(),
        Span::raw(clock),
    ];
    if frozen {
        first.push(Span::raw("  ❄ boards frozen").light_blue());
    }
    if let Some(me) = &state.me {
        first.push(Span::raw(format!("   you: {me}")).dark_gray());
    }
    let selected = state.selected();
    let problems: Vec<Span> = state
        .problems()
        .into_iter()
        .map(|p| {
            let style = if Some(&p) == selected.as_ref() {
                Style::new().reversed()
            } else {
                Style::new()
            };
            Span::styled(format!(" {p} "), style)
        })
        .collect();
    vec![Line::from(first), Line::from(problems)]
}

fn draw_table(f: &mut Frame, area: Rect, state: &State, board: Option<&Board>, by_time: bool) {
    let title = if by_time { " Fastest " } else { " Leanest " };
    let block = Block::bordered().title(title);
    let Some(board) = board else {
        f.render_widget(Paragraph::new("No results yet.").block(block), area);
        return;
    };
    let entries: &[BoardEntry] = if by_time {
        &board.by_time
    } else {
        &board.by_memory
    };
    let rows = entries.iter().map(|e| {
        let row = Row::new(vec![
            Cell::from(format!("{:>3}", e.rank)),
            Cell::from(e.display_name.clone()),
            Cell::from(format!("{:>9}", format::time(e.time_us))),
            Cell::from(format!("{:>9}", format::memory(e.memory_kib))),
            Cell::from(format!("{:>4}", e.attempts)),
        ]);
        if state.me.as_deref() == Some(e.user.as_str()) {
            row.style(Style::new().add_modifier(Modifier::BOLD).fg(Color::Yellow))
        } else {
            row
        }
    });
    let header = Row::new(vec!["  #", "name", "     time", "   memory", "subs"]).bold();
    let table = Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Min(12),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(5),
        ],
    )
    .header(header)
    .block(block);
    f.render_widget(table, area);
}

fn draw_scatter(f: &mut Frame, area: Rect, board: Option<&Board>) {
    let block = Block::bordered().title(" Best time vs. memory (each participant's fastest) ");
    let Some(board) = board.filter(|b| !b.by_time.is_empty()) else {
        f.render_widget(Paragraph::new("No results yet.").block(block), area);
        return;
    };
    let points: Vec<Vec<(f64, f64)>> = board
        .by_time
        .iter()
        .map(|e| vec![(e.time_us as f64 / 1000.0, e.memory_kib as f64 / 1024.0)])
        .collect();
    let datasets = board
        .by_time
        .iter()
        .zip(&points)
        .enumerate()
        .map(|(i, (e, p))| {
            Dataset::default()
                .name(e.display_name.clone())
                .marker(Marker::Dot)
                .graph_type(GraphType::Scatter)
                .style(Style::new().fg(PALETTE[i % PALETTE.len()]))
                .data(p)
        })
        .collect();
    let (x_max, y_max) = board.by_time.iter().fold((1.0f64, 1.0f64), |(x, y), e| {
        (
            x.max(e.time_us as f64 / 1000.0),
            y.max(e.memory_kib as f64 / 1024.0),
        )
    });
    let (x_max, y_max) = (x_max * 1.1, y_max * 1.1);
    let chart = Chart::new(datasets)
        .block(block)
        .x_axis(
            Axis::default()
                .title("CPU time (ms)")
                .bounds([0.0, x_max])
                .labels([
                    "0".to_owned(),
                    format!("{:.0}", x_max / 2.0),
                    format!("{x_max:.0}"),
                ]),
        )
        .y_axis(Axis::default().title("MiB").bounds([0.0, y_max]).labels([
            "0".to_owned(),
            format!("{:.1}", y_max / 2.0),
            format!("{y_max:.1}"),
        ]));
    f.render_widget(chart, area);
}

/// Each participant's time-board rank after every accepted submission, in minutes
/// since the round started. Ranks are negated so rank 1 is at the top.
pub fn rank_series(
    history: &[AcceptedSubmission],
    start: i64,
) -> BTreeMap<String, Vec<(f64, f64)>> {
    let mut best: HashMap<&str, (u64, u64, i64)> = HashMap::new();
    let mut series: BTreeMap<String, Vec<(f64, f64)>> = BTreeMap::new();
    for a in history {
        let key = (a.time_us, a.memory_kib, a.submission_id);
        let entry = best.entry(&a.user).or_insert(key);
        if key < *entry {
            *entry = key;
        }
        let mut order: Vec<(&str, (u64, u64, i64))> = best.iter().map(|(u, k)| (*u, *k)).collect();
        order.sort_by_key(|(_, k)| *k);
        let t = (a.submitted_at - start).max(0) as f64 / 60.0;
        for (rank, (user, _)) in order.iter().enumerate() {
            series
                .entry(user.to_string())
                .or_default()
                .push((t, -(rank as f64 + 1.0)));
        }
    }
    series
}

fn draw_ranks(f: &mut Frame, area: Rect, state: &State, problem: Option<&str>) {
    let block = Block::bordered().title(" Rank by CPU time over the round ");
    let history = problem.and_then(|p| state.history.get(p));
    let (Some(history), Some(round)) = (history.filter(|h| !h.is_empty()), &state.round) else {
        f.render_widget(Paragraph::new("No results yet.").block(block), area);
        return;
    };
    let start = round.started_at.unwrap_or(0);
    let series = rank_series(history, start);
    let n = series.len().max(1) as f64;
    let end = (state.now().min(round.ends_at.unwrap_or(i64::MAX)) - start).max(60) as f64 / 60.0;
    let names: HashMap<&str, &str> = history
        .iter()
        .map(|a| (a.user.as_str(), a.display_name.as_str()))
        .collect();
    let datasets = series
        .iter()
        .enumerate()
        .map(|(i, (user, points))| {
            Dataset::default()
                .name(names.get(user.as_str()).copied().unwrap_or(user).to_owned())
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::new().fg(PALETTE[i % PALETTE.len()]))
                .data(points)
        })
        .collect();
    let chart = Chart::new(datasets)
        .block(block)
        .x_axis(Axis::default().title("minutes").bounds([0.0, end]).labels([
            "0".to_owned(),
            format!("{:.0}", end / 2.0),
            format!("{end:.0}"),
        ]))
        .y_axis(
            Axis::default()
                .title("rank")
                .bounds([-n - 0.5, -0.5])
                .labels([format!("{n:.0}"), "1".to_owned()]),
        );
    f.render_widget(chart, area);
}

fn draw_mine(f: &mut Frame, area: Rect, state: &State) {
    let block = Block::bordered().title(" My submissions ");
    if state.me.is_none() {
        f.render_widget(
            Paragraph::new("Log in (`arena login`) to see your submissions here.").block(block),
            area,
        );
        return;
    }
    let rows = state.mine.iter().rev().map(|s| {
        let verdict = match s.status {
            SubmissionStatus::Queued => "queued",
            SubmissionStatus::Judging => "judging",
            _ => s.verdict.map(format::verdict).unwrap_or("?"),
        };
        let color = match s.verdict {
            Some(Verdict::Accepted) => Color::Green,
            Some(_) => Color::Red,
            None => Color::Gray,
        };
        let failed = s
            .report
            .as_ref()
            .and_then(|r| r.first_failure())
            .map(|t| format!("test {}", t.name))
            .unwrap_or_default();
        let (time, mem) = match (s.verdict, s.time_us, s.memory_kib) {
            (Some(Verdict::Accepted), Some(t), Some(m)) => (format::time(t), format::memory(m)),
            _ => (String::new(), String::new()),
        };
        Row::new(vec![
            Cell::from(format!("{:>5}", s.id)),
            Cell::from(s.problem.clone()),
            Cell::from(verdict).style(Style::new().fg(color)),
            Cell::from(failed),
            Cell::from(format!("{time:>9}")),
            Cell::from(format!("{mem:>9}")),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(6),
            Constraint::Min(10),
            Constraint::Length(22),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
        ],
    )
    .header(
        Row::new(vec![
            "   id",
            "problem",
            "verdict",
            "failed",
            "     time",
            "   memory",
        ])
        .bold(),
    )
    .block(block);
    f.render_widget(table, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use arena_protocol::ProblemSummary;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn entry(rank: u32, user: &str, time_us: u64, memory_kib: u64) -> BoardEntry {
        BoardEntry {
            rank,
            user: user.into(),
            display_name: user.to_uppercase(),
            time_us,
            memory_kib,
            submission_id: rank as i64,
            submitted_at: 0,
            attempts: 2,
        }
    }

    fn sample_state() -> State {
        let mut s = State::default();
        s.apply(ServerEvent::Hello {
            user: Some("ann".into()),
        });
        s.apply(ServerEvent::Round {
            round: RoundInfo {
                id: 1,
                name: "Final".into(),
                state: RoundState::Running,
                duration_minutes: 60,
                freeze_minutes: 0,
                cooldown_seconds: 30,
                started_at: Some(format::now() - 600),
                ends_at: Some(format::now() + 3000),
                problems: vec![ProblemSummary {
                    slug: "sum".into(),
                    title: "Sum".into(),
                    version: 1,
                    time_limit_ms: 1000,
                    memory_limit_mb: 256,
                    test_count: 2,
                }],
                server_time: format::now(),
                queue: None,
            },
        });
        s.apply(ServerEvent::Board {
            board: Board {
                round_id: 1,
                problem: "sum".into(),
                frozen: false,
                by_time: vec![entry(1, "ann", 12_000, 1500), entry(2, "bob", 40_000, 900)],
                by_memory: vec![entry(1, "bob", 40_000, 900), entry(2, "ann", 12_000, 1500)],
            },
        });
        s
    }

    fn render(state: &State) -> String {
        let mut t = Terminal::new(TestBackend::new(100, 24)).unwrap();
        t.draw(|f| draw(f, state)).unwrap();
        let buf = t.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn time_and_memory_tabs_list_participants() {
        let mut s = sample_state();
        let screen = render(&s);
        assert!(screen.contains("Final"), "{screen}");
        assert!(
            screen.contains("ANN") && screen.contains("12 ms"),
            "{screen}"
        );

        handle_key(&mut s, KeyCode::Char('2'), KeyModifiers::NONE);
        let screen = render(&s);
        let (bob, ann) = (screen.find("BOB").unwrap(), screen.find("ANN").unwrap());
        assert!(bob < ann, "bob is leaner:\n{screen}");
    }

    #[test]
    fn every_tab_renders() {
        let mut s = sample_state();
        s.history.insert(
            "sum".into(),
            vec![AcceptedSubmission {
                submission_id: 1,
                user: "ann".into(),
                display_name: "ANN".into(),
                time_us: 12_000,
                memory_kib: 1500,
                submitted_at: format::now() - 300,
            }],
        );
        for key in ['3', '4', '5'] {
            handle_key(&mut s, KeyCode::Char(key), KeyModifiers::NONE);
            render(&s);
        }
        assert!(handle_key(&mut s, KeyCode::Char('q'), KeyModifiers::NONE));
    }

    #[test]
    fn rank_over_time_replays_improvements() {
        let a = |id, user: &str, time_us, at| AcceptedSubmission {
            submission_id: id,
            user: user.into(),
            display_name: user.into(),
            time_us,
            memory_kib: 1,
            submitted_at: at,
        };
        let history = vec![
            a(1, "ann", 50, 60),
            a(2, "bob", 40, 120),
            a(3, "ann", 30, 180),
        ];
        let s = rank_series(&history, 0);
        // ann: 1st, then 2nd when bob arrives, then 1st again.
        let ann: Vec<f64> = s["ann"].iter().map(|p| -p.1).collect();
        assert_eq!(ann, [1.0, 2.0, 1.0]);
        let bob: Vec<f64> = s["bob"].iter().map(|p| -p.1).collect();
        assert_eq!(bob, [1.0, 2.0]);
        assert_eq!(s["ann"][2].0, 3.0, "minutes since start");
    }
}
