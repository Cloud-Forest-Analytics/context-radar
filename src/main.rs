use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};
use uuid::Uuid;

#[derive(Parser)]
#[command(name = "context-radar")]
#[command(about = "Monitor, summarize, disentangle, and curate Claude Code session context.")]
struct Cli {
    #[arg(long)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    InitConfig,
    Scan(ScanArgs),
    Summarize(SummarizeArgs),
    AuthorContext(AuthorContextArgs),
    Curate(CurateArgs),
    Kickoff(KickoffArgs),
    StationAdd(StationAddArgs),
    StationMonthly(StationMonthlyArgs),
    TopicPack(TopicPackArgs),
    Watch(WatchArgs),
}

#[derive(Args)]
struct ScanArgs {
    #[arg(long, default_value_t = false)]
    json: bool,
}

#[derive(Args)]
struct SummarizeArgs {
    #[arg(long, default_value = "reports/latest-session-digest.md")]
    output: PathBuf,
    #[arg(long, default_value = "reports/latest-session-summaries.json")]
    json_output: PathBuf,
}

#[derive(Args)]
struct AuthorContextArgs {
    #[arg(long)]
    session_id: Option<String>,
    #[arg(long, default_value_t = 40)]
    max_turns: usize,
    #[arg(long, default_value = "reports/latest-authored-context-window.md")]
    output: PathBuf,
    #[arg(long, default_value = "reports/latest-authored-context-window.json")]
    json_output: PathBuf,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Horizon {
    ShortTerm,
    LongTerm,
}

#[derive(Args)]
struct CurateArgs {
    #[arg(long)]
    title: String,
    #[arg(long, value_enum)]
    horizon: Horizon,
    #[arg(long)]
    context_file: PathBuf,
    #[arg(long)]
    source_session_id: Option<String>,
    #[arg(long)]
    project_cwd: Option<String>,
    #[arg(long, default_value = "")]
    tags: String,
    #[arg(long, default_value = "")]
    notes: String,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum HorizonFilter {
    ShortTerm,
    LongTerm,
    Both,
}

#[derive(Args)]
struct KickoffArgs {
    #[arg(long, value_enum, default_value_t = HorizonFilter::Both)]
    horizon: HorizonFilter,
    #[arg(long, default_value = "")]
    ids: String,
    #[arg(long, default_value = "reports/kickoff-context.md")]
    output: PathBuf,
    #[arg(long, default_value_t = 18000)]
    max_chars: usize,
}

#[derive(Args)]
struct StationAddArgs {
    #[arg(long)]
    repo: String,
    #[arg(long, value_enum)]
    horizon: Horizon,
    #[arg(long)]
    title: String,
    #[arg(long)]
    summary_file: PathBuf,
    #[arg(long, default_value = "")]
    tags: String,
}

#[derive(Args)]
struct StationMonthlyArgs {
    #[arg(long)]
    repo: String,
    #[arg(long)]
    month: String,
    #[arg(long, value_enum, default_value_t = Horizon::LongTerm)]
    horizon: Horizon,
    #[arg(long, default_value = "reports/station-monthly.md")]
    output: PathBuf,
}

#[derive(Args)]
struct WatchArgs {
    #[arg(long, default_value_t = 10)]
    interval_secs: u64,
    #[arg(long)]
    repo: Option<PathBuf>,
    #[arg(long, default_value_t = 40)]
    max_turns: usize,
    #[arg(long, default_value = "reports/watch-latest.md")]
    output: PathBuf,
}

#[derive(Args)]
struct TopicPackArgs {
    #[arg(long)]
    repo_cwd: PathBuf,
    #[arg(long)]
    output_root: Option<PathBuf>,
    #[arg(long, default_value_t = 8)]
    max_sessions: usize,
    #[arg(long, default_value_t = 40)]
    max_turns: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppConfig {
    sessions_root: String,
    watch_roots: Vec<String>,
    max_sessions: usize,
    max_turns_per_session: usize,
    aggregate_title: String,
    memory_catalog_path: String,
    station_root: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            sessions_root: "~/.claude/projects".to_string(),
            watch_roots: vec!["~/projects".to_string(), "/mnt/c/Users/pabto/projects".to_string()],
            max_sessions: 12,
            max_turns_per_session: 8,
            aggregate_title: "Claude Code Session Digest".to_string(),
            memory_catalog_path: "data/memory/catalog.json".to_string(),
            station_root: "data/stations".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct SessionTurn {
    role: String,
    text: String,
    timestamp: String,
}

#[derive(Debug, Clone, Serialize)]
struct SessionRecord {
    session_id: String,
    file_path: PathBuf,
    project_cwd: PathBuf,
    updated_at: DateTime<Utc>,
    turns: Vec<SessionTurn>,
}

#[derive(Debug, Clone, Serialize)]
struct SessionSummary {
    session_id: String,
    project_cwd: String,
    updated_at: String,
    summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CuratedContext {
    id: String,
    title: String,
    horizon: Horizon,
    tags: Vec<String>,
    source_session_id: Option<String>,
    project_cwd: Option<String>,
    context_path: String,
    notes: String,
    created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MemoryCatalog {
    version: u32,
    entries: Vec<CuratedContext>,
}

impl Default for MemoryCatalog {
    fn default() -> Self {
        Self {
            version: 1,
            entries: Vec::new(),
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config_path = cli.config.unwrap_or_else(|| PathBuf::from("context-radar.config.json"));

    match cli.command {
        Commands::InitConfig => {
            write_default_config(&config_path)?;
            println!("Wrote config: {}", config_path.display());
            Ok(())
        }
        cmd => {
            let cfg = load_config(&config_path)?;
            match cmd {
                Commands::Scan(args) => cmd_scan(&cfg, args),
                Commands::Summarize(args) => cmd_summarize(&cfg, args),
                Commands::AuthorContext(args) => cmd_author_context(&cfg, args),
                Commands::Curate(args) => cmd_curate(&cfg, args),
                Commands::Kickoff(args) => cmd_kickoff(&cfg, args),
                Commands::StationAdd(args) => cmd_station_add(&cfg, args),
                Commands::StationMonthly(args) => cmd_station_monthly(&cfg, args),
                Commands::TopicPack(args) => cmd_topic_pack(&cfg, args),
                Commands::Watch(args) => cmd_watch(&cfg, args),
                Commands::InitConfig => Ok(()),
            }
        }
    }
}

fn cmd_scan(cfg: &AppConfig, args: ScanArgs) -> Result<()> {
    let sessions = discover_sessions(
        cfg,
        cfg.max_sessions,
        cfg.max_turns_per_session,
        None::<&str>,
        false,
    )?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&sessions)?);
        return Ok(());
    }
    if sessions.is_empty() {
        println!("No Claude Code session logs found under watch roots.");
        return Ok(());
    }
    println!("Found {} session logs:", sessions.len());
    for session in sessions {
        println!(
            "- {} | {} | {}",
            session.session_id,
            session.project_cwd.display(),
            session.file_path.display()
        );
    }
    Ok(())
}

fn cmd_summarize(cfg: &AppConfig, args: SummarizeArgs) -> Result<()> {
    let sessions = discover_sessions(
        cfg,
        cfg.max_sessions,
        cfg.max_turns_per_session,
        None::<&str>,
        false,
    )?;
    let mut summaries = Vec::new();
    for session in &sessions {
        summaries.push(summarize_session(session)?);
    }
    let digest = aggregate_summaries(&cfg.aggregate_title, &summaries)?;
    write_text_file(&args.output, &digest)?;
    write_json_file(&args.json_output, &summaries)?;
    println!("Wrote digest: {}", args.output.display());
    println!("Wrote raw summaries: {}", args.json_output.display());
    Ok(())
}

fn cmd_author_context(cfg: &AppConfig, args: AuthorContextArgs) -> Result<()> {
    let sessions = discover_sessions(cfg, 50.max(cfg.max_sessions), args.max_turns, args.session_id.as_deref(), true)?;
    let target = sessions
        .first()
        .ok_or_else(|| anyhow!("No matching sessions found under watch roots."))?;

    let topics = disentangle_topics(target)?;
    let entropy = extract_entropy_summary(target)?;
    let authored = build_authored_context_window(target, &topics, &entropy)?;

    let payload = format!(
        "# Session Context Package\n\n- session_id: `{}`\n- project_cwd: `{}`\n- updated_at: `{}`\n\n{}\n\n{}\n\n{}\n",
        target.session_id,
        target.project_cwd.display(),
        target.updated_at.to_rfc3339(),
        topics,
        entropy,
        authored
    );
    write_text_file(&args.output, &payload)?;
    write_json_file(
        &args.json_output,
        &serde_json::json!({
            "session_id": target.session_id,
            "project_cwd": target.project_cwd,
            "updated_at": target.updated_at.to_rfc3339(),
            "topics_markdown": topics,
            "entropy_markdown": entropy,
            "authored_window_markdown": authored
        }),
    )?;
    println!("Wrote authored context window: {}", args.output.display());
    println!("Wrote authored context JSON: {}", args.json_output.display());
    Ok(())
}

fn cmd_curate(cfg: &AppConfig, args: CurateArgs) -> Result<()> {
    let catalog_path = expand_path(&cfg.memory_catalog_path);
    let mut catalog = load_catalog(&catalog_path)?;
    let tags = args
        .tags
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    let entry = CuratedContext {
        id: Uuid::new_v4().to_string(),
        title: args.title,
        horizon: args.horizon,
        tags,
        source_session_id: args.source_session_id,
        project_cwd: args.project_cwd,
        context_path: args.context_file.to_string_lossy().to_string(),
        notes: args.notes,
        created_at: Utc::now().to_rfc3339(),
    };
    catalog.entries.push(entry.clone());
    save_catalog(&catalog_path, &catalog)?;
    println!("Curated context added:");
    println!("- id: {}", entry.id);
    println!("- title: {}", entry.title);
    println!("- horizon: {:?}", entry.horizon);
    println!("- catalog: {}", catalog_path.display());
    Ok(())
}

fn cmd_kickoff(cfg: &AppConfig, args: KickoffArgs) -> Result<()> {
    let catalog_path = expand_path(&cfg.memory_catalog_path);
    let catalog = load_catalog(&catalog_path)?;

    let selected_ids = args
        .ids
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();

    let mut selected = catalog
        .entries
        .iter()
        .filter(|entry| matches_horizon(entry.horizon, args.horizon))
        .collect::<Vec<_>>();

    if !selected_ids.is_empty() {
        selected.retain(|entry| selected_ids.contains(&entry.id.as_str()));
    }
    if selected.is_empty() {
        bail!("No curated contexts match filters.");
    }

    let mut combined = String::new();
    combined.push_str("# Context-Radar Kickoff Packet\n\n");
    combined.push_str("## Included Contexts\n");
    for entry in &selected {
        combined.push_str(&format!(
            "- {} | {} | {:?} | tags: {}\n",
            entry.id,
            entry.title,
            entry.horizon,
            entry.tags.join(",")
        ));
    }
    combined.push_str("\n## Curated Context Bodies\n");
    for entry in &selected {
        combined.push_str(&format!("\n### {} ({})\n", entry.title, entry.id));
        let body = fs::read_to_string(&entry.context_path).unwrap_or_else(|_| {
            format!(
                "Context file unavailable at `{}`. Keep entry metadata only.",
                entry.context_path
            )
        });
        combined.push_str(&truncate_chars(&body, args.max_chars / selected.len().max(1)));
        combined.push('\n');
    }
    combined.push_str("\n## Ready-to-run non-interactive command\n");
    combined.push_str("Use the generated markdown as context input:\n");
    combined.push_str("`claude -p --model sonnet \"<paste mission + selected context packet>\"`\n");

    write_text_file(&args.output, &combined)?;
    println!("Wrote kickoff packet: {}", args.output.display());
    println!("Catalog used: {}", catalog_path.display());
    Ok(())
}

fn cmd_station_add(cfg: &AppConfig, args: StationAddArgs) -> Result<()> {
    let station_root = expand_path(&cfg.station_root);
    let lane = horizon_lane(args.horizon);
    let repo_slug = slugify(&args.repo);
    let title_slug = slugify(&args.title);
    let timestamp = Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let station_file = station_root
        .join(&repo_slug)
        .join(lane)
        .join(format!("{timestamp}-{title_slug}.md"));

    let summary = fs::read_to_string(&args.summary_file)
        .with_context(|| format!("failed to read summary file {}", args.summary_file.display()))?;
    let tags = split_tags(&args.tags);
    let body = format!(
        "# {}\n\n- repo: `{}`\n- lane: `{}`\n- created_at: `{}`\n- tags: `{}`\n\n{}\n",
        args.title,
        args.repo,
        lane,
        Utc::now().to_rfc3339(),
        tags.join(","),
        summary
    );
    write_text_file(&station_file, &body)?;

    println!("Wrote station memory: {}", station_file.display());
    Ok(())
}

fn cmd_station_monthly(cfg: &AppConfig, args: StationMonthlyArgs) -> Result<()> {
    let station_root = expand_path(&cfg.station_root);
    let repo_slug = slugify(&args.repo);
    let lane = horizon_lane(args.horizon);
    let lane_dir = station_root.join(&repo_slug).join(lane);
    if !lane_dir.exists() {
        bail!("No station lane directory found at {}", lane_dir.display());
    }

    let mut entries = Vec::new();
    for item in fs::read_dir(&lane_dir)? {
        let path = item?.path();
        if path.extension() != Some(OsStr::new("md")) {
            continue;
        }
        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if !filename.starts_with(&args.month.replace('-', "")) && !args.month.is_empty() {
            continue;
        }
        let content = fs::read_to_string(&path).unwrap_or_default();
        if !content.trim().is_empty() {
            entries.push((path, content));
        }
    }

    if entries.is_empty() {
        bail!(
            "No station summaries found for repo={} month={} lane={}",
            args.repo,
            args.month,
            lane
        );
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut corpus = String::new();
    for (path, content) in &entries {
        corpus.push_str(&format!("## Source: {}\n{}\n\n", path.display(), content));
    }

    let prompt = format!(
        "You are generating a monthly knowledge-management deep dive from curated engineering memory.\n\
         Keep this strategic, not operational metrics.\n\
         Return markdown with sections:\n\
         # Monthly Knowledge Deep Dive\n\
         ## Core Themes\n\
         ## Durable Decisions\n\
         ## Open Questions Worth Investment\n\
         ## Suggested Curations for Next Month\n\
         Be concrete and high entropy. No fluff.\n\n\
         Repo: {}\n\
         Month: {}\n\
         Lane: {}\n\
         Curated memory corpus:\n{}\n",
        args.repo, args.month, lane, corpus
    );
    let monthly = run_haiku(&prompt)?;

    let default_output = station_root
        .join(&repo_slug)
        .join("long-term")
        .join("monthly")
        .join(format!("{}.md", args.month));
    let output_path = if args.output == PathBuf::from("reports/station-monthly.md") {
        default_output
    } else {
        args.output
    };
    write_text_file(&output_path, &monthly)?;
    println!("Wrote monthly deep dive: {}", output_path.display());
    Ok(())
}

fn cmd_topic_pack(cfg: &AppConfig, args: TopicPackArgs) -> Result<()> {
    let canonical_repo = fs::canonicalize(&args.repo_cwd)
        .with_context(|| format!("repo path not found: {}", args.repo_cwd.display()))?;
    let repo_name = canonical_repo
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("repo");
    let output_root = args
        .output_root
        .unwrap_or_else(|| canonical_repo.join("docs/context-memory"));
    let raw_dir = output_root.join("raw");
    let topics_dir = output_root.join("topics");
    fs::create_dir_all(&raw_dir)?;
    fs::create_dir_all(&topics_dir)?;

    let sessions = discover_sessions_for_repo(cfg, &canonical_repo, args.max_sessions, args.max_turns)?;
    if sessions.is_empty() {
        bail!(
            "No sessions found for repo {} under watch roots/sessions store.",
            canonical_repo.display()
        );
    }

    let mut topic_map_lines = Vec::new();
    topic_map_lines.push("# Topic Pack".to_string());
    topic_map_lines.push(String::new());
    topic_map_lines.push(format!("- repo: `{}`", canonical_repo.display()));
    topic_map_lines.push(format!("- generated_at: `{}`", Utc::now().to_rfc3339()));
    topic_map_lines.push(String::new());
    topic_map_lines.push("## Sessions".to_string());
    topic_map_lines.push(String::new());

    let mut generated_count = 0usize;
    let mut failed = Vec::new();
    for session in &sessions {
        match authored_context_artifacts(session) {
            Ok((topics, entropy, authored)) => {
                let pack = format!(
                    "# Session Context Package\n\n- session_id: `{}`\n- project_cwd: `{}`\n- updated_at: `{}`\n\n{}\n\n{}\n\n{}\n",
                    session.session_id,
                    session.project_cwd.display(),
                    session.updated_at.to_rfc3339(),
                    topics,
                    entropy,
                    authored
                );
                let md_path = raw_dir.join(format!("{}.md", session.session_id));
                let json_path = raw_dir.join(format!("{}.json", session.session_id));
                write_text_file(&md_path, &pack)?;
                write_json_file(
                    &json_path,
                    &serde_json::json!({
                        "session_id": session.session_id,
                        "project_cwd": session.project_cwd,
                        "updated_at": session.updated_at.to_rfc3339(),
                        "topics_markdown": topics,
                        "entropy_markdown": entropy,
                        "authored_window_markdown": authored
                    }),
                )?;

                let topic_slug = topic_slug_from_markdown(&pack);
                remove_stale_topic_copies(&topics_dir, &session.session_id)?;
                let topic_bucket = topics_dir.join(&topic_slug);
                fs::create_dir_all(&topic_bucket)?;
                let topic_copy = topic_bucket.join(format!("{}.md", session.session_id));
                fs::copy(&md_path, &topic_copy).with_context(|| {
                    format!(
                        "failed to copy session pack {} to topic bucket {}",
                        md_path.display(),
                        topic_copy.display()
                    )
                })?;

                topic_map_lines.push(format!(
                    "- `{}` -> `topics/{}/{}.md`",
                    session.session_id, topic_slug, session.session_id
                ));
                generated_count += 1;
            }
            Err(err) => {
                failed.push(format!("{} ({})", session.session_id, err));
            }
        }
    }

    if !failed.is_empty() {
        topic_map_lines.push(String::new());
        topic_map_lines.push("## Failures".to_string());
        topic_map_lines.push(String::new());
        for row in failed {
            topic_map_lines.push(format!("- {}", row));
        }
    }

    let map_path = output_root.join("TOPIC-MAP.md");
    write_text_file(&map_path, &topic_map_lines.join("\n"))?;
    println!(
        "Topic pack generated for {}: {} sessions processed into {}",
        repo_name,
        generated_count,
        output_root.display()
    );
    Ok(())
}

fn cmd_watch(cfg: &AppConfig, args: WatchArgs) -> Result<()> {
    let sessions_root = expand_path(&cfg.sessions_root);
    if !sessions_root.exists() {
        bail!("sessions_root not found: {}", sessions_root.display());
    }
    let watch_roots: Vec<PathBuf> = cfg.watch_roots.iter().map(|p| expand_path(p)).collect();
    let repo_filter = args
        .repo
        .as_deref()
        .map(fs::canonicalize)
        .transpose()
        .unwrap_or(None);

    println!(
        "Watching {} every {}s — Ctrl-C to stop.",
        sessions_root.display(),
        args.interval_secs
    );
    if let Some(ref r) = repo_filter {
        println!("  repo filter: {}", r.display());
    }

    let mut seen: HashMap<PathBuf, SystemTime> = HashMap::new();

    loop {
        let snapshot = collect_jsonl_mtimes(&sessions_root)?;
        let changed: Vec<PathBuf> = snapshot
            .iter()
            .filter(|(path, mtime)| seen.get(*path).map(|old| old != *mtime).unwrap_or(true))
            .map(|(path, _)| path.clone())
            .collect();

        for path in &changed {
            let Ok(Some(record)) = parse_session_file(path, args.max_turns) else {
                continue;
            };
            if !is_under_watch(&record.project_cwd, &watch_roots) {
                continue;
            }
            if let Some(ref canonical_repo) = repo_filter {
                let matches = fs::canonicalize(&record.project_cwd)
                    .map(|p| p == *canonical_repo)
                    .unwrap_or(false);
                if !matches {
                    continue;
                }
            }

            println!(
                "[watch] changed: {} ({})",
                record.session_id,
                record.project_cwd.display()
            );

            match authored_context_artifacts(&record) {
                Ok((topics, entropy, authored)) => {
                    let payload = format!(
                        "# Watch Snapshot\n\n- session_id: `{}`\n- project_cwd: `{}`\n- updated_at: `{}`\n\n{}\n\n{}\n\n{}\n",
                        record.session_id,
                        record.project_cwd.display(),
                        record.updated_at.to_rfc3339(),
                        topics,
                        entropy,
                        authored
                    );
                    if let Err(e) = write_text_file(&args.output, &payload) {
                        eprintln!("[watch] write failed: {e}");
                    } else {
                        println!("[watch] wrote {}", args.output.display());
                    }
                }
                Err(e) => eprintln!("[watch] pipeline failed for {}: {e}", record.session_id),
            }
        }

        seen = snapshot;
        std::thread::sleep(Duration::from_secs(args.interval_secs));
    }
}

fn collect_jsonl_mtimes(sessions_root: &Path) -> Result<HashMap<PathBuf, SystemTime>> {
    let mut map = HashMap::new();
    for project_dir in fs::read_dir(sessions_root)? {
        let project_dir = project_dir?;
        if !project_dir.path().is_dir() {
            continue;
        }
        for file in fs::read_dir(project_dir.path())? {
            let file = file?;
            let path = file.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            if let Ok(meta) = fs::metadata(&path) {
                if let Ok(mtime) = meta.modified() {
                    map.insert(path, mtime);
                }
            }
        }
    }
    Ok(map)
}

fn matches_horizon(entry: Horizon, filter: HorizonFilter) -> bool {
    matches!(
        (entry, filter),
        (_, HorizonFilter::Both)
            | (Horizon::ShortTerm, HorizonFilter::ShortTerm)
            | (Horizon::LongTerm, HorizonFilter::LongTerm)
    )
}

fn summarize_session(session: &SessionRecord) -> Result<SessionSummary> {
    let prompt = format!(
        "You are summarizing Claude Code session logs for engineering context carryover.\n\
         Return 3 bullets max:\n\
         - main objective\n\
         - concrete outcomes\n\
         - next action or blocker\n\n\
         Session ID: {}\n\
         Project cwd: {}\n\
         Updated at UTC: {}\n\
         Recent turns:\n{}\n",
        session.session_id,
        session.project_cwd.display(),
        session.updated_at.to_rfc3339(),
        turns_as_text(session)
    );
    let summary = run_haiku(&prompt)?;
    Ok(SessionSummary {
        session_id: session.session_id.clone(),
        project_cwd: session.project_cwd.display().to_string(),
        updated_at: session.updated_at.to_rfc3339(),
        summary,
    })
}

fn aggregate_summaries(title: &str, summaries: &[SessionSummary]) -> Result<String> {
    if summaries.is_empty() {
        return Ok(format!("# {}\n\nNo sessions found under watch roots.\n", title));
    }
    let payload = serde_json::to_string_pretty(summaries)?;
    let prompt = format!(
        "Create a concise engineering session digest from these per-session summaries.\n\
         Output in markdown with sections:\n\
         1) What moved today\n\
         2) Active blockers\n\
         3) Suggested manager actions next\n\
         Be specific; no fluff.\n\n\
         Digest title: {}\n\
         Summaries JSON:\n{}\n",
        title, payload
    );
    run_haiku(&prompt)
}

fn disentangle_topics(session: &SessionRecord) -> Result<String> {
    let prompt = format!(
        "Disentangle mixed topics from this engineering conversation.\n\
         Return markdown only.\n\
         Format:\n\
         ## Topic Threads\n\
         For each topic: heading, objective, key evidence, status.\n\
         Include a final section: Cross-topic collisions.\n\n\
         Session ID: {}\n\
         Project cwd: {}\n\
         Turns:\n{}\n",
        session.session_id,
        session.project_cwd.display(),
        turns_as_text(session)
    );
    run_haiku(&prompt)
}

fn extract_entropy_summary(session: &SessionRecord) -> Result<String> {
    let prompt = format!(
        "Extract a high-entropy summary from this engineering conversation.\n\
         Return markdown only with sections in this exact order:\n\
         ## High-Entropy Facts\n\
         ## Decisions and Rationale\n\
         ## Open Loops\n\
         ## Risks and Unknowns\n\
         ## Reusable Constraints\n\
         Use concrete, specific bullets only.\n\n\
         Session ID: {}\n\
         Project cwd: {}\n\
         Turns:\n{}\n",
        session.session_id,
        session.project_cwd.display(),
        turns_as_text(session)
    );
    run_haiku(&prompt)
}

fn build_authored_context_window(session: &SessionRecord, topics: &str, entropy: &str) -> Result<String> {
    let prompt = format!(
        "Create an authored context window for bootstrapping a new Claude working session.\n\
         Return markdown only.\n\
         Sections:\n\
         # Authored Context Window\n\
         ## Mission\n\
         ## Current State\n\
         ## Critical Constraints\n\
         ## Immediate Next Actions\n\
         ## Ready-to-send Kickoff Prompt\n\n\
         Session metadata:\n\
         - session_id: {}\n\
         - project_cwd: {}\n\
         - updated_at_utc: {}\n\n\
         Topic disentangling:\n{}\n\n\
         High-entropy summary:\n{}\n",
        session.session_id,
        session.project_cwd.display(),
        session.updated_at.to_rfc3339(),
        topics,
        entropy
    );
    run_haiku(&prompt)
}

fn authored_context_artifacts(session: &SessionRecord) -> Result<(String, String, String)> {
    let topics = disentangle_topics(session)?;
    let entropy = extract_entropy_summary(session)?;
    let authored = build_authored_context_window(session, &topics, &entropy)?;
    Ok((topics, entropy, authored))
}

fn run_haiku(prompt: &str) -> Result<String> {
    let output = Command::new("claude")
        .args(["-p", "--model", "haiku", "--output-format", "text", prompt])
        .output()
        .context("failed to execute claude CLI for Haiku summarization")?;
    if !output.status.success() {
        bail!(
            "haiku call failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn discover_sessions(
    cfg: &AppConfig,
    max_sessions: usize,
    max_turns: usize,
    session_id: Option<&str>,
    strict_target: bool,
) -> Result<Vec<SessionRecord>> {
    let sessions_root = expand_path(&cfg.sessions_root);
    if !sessions_root.exists() {
        return Ok(Vec::new());
    }
    let watch_roots = cfg
        .watch_roots
        .iter()
        .map(|p| expand_path(p))
        .collect::<Vec<_>>();
    let mut records = Vec::new();

    for project_dir in fs::read_dir(&sessions_root)? {
        let project_dir = project_dir?;
        if !project_dir.path().is_dir() {
            continue;
        }
        for file in fs::read_dir(project_dir.path())? {
            let file = file?;
            let path = file.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let parsed = parse_session_file(&path, max_turns)?;
            let Some(parsed) = parsed else { continue };
            if let Some(target_id) = session_id {
                if parsed.session_id != target_id {
                    continue;
                }
            }
            if is_under_watch(&parsed.project_cwd, &watch_roots) {
                records.push(parsed);
            }
        }
    }
    if strict_target && session_id.is_some() && records.is_empty() {
        bail!("session not found in watch roots");
    }

    records.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    records.truncate(max_sessions);
    Ok(records)
}

fn discover_sessions_for_repo(
    cfg: &AppConfig,
    repo_cwd: &Path,
    max_sessions: usize,
    max_turns: usize,
) -> Result<Vec<SessionRecord>> {
    let mut sessions = discover_sessions(cfg, max_sessions.saturating_mul(5).max(50), max_turns, None::<&str>, false)?;
    let canonical_repo = fs::canonicalize(repo_cwd)?;
    sessions.retain(|s| {
        fs::canonicalize(&s.project_cwd)
            .map(|p| p == canonical_repo)
            .unwrap_or(false)
    });
    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    sessions.truncate(max_sessions);
    Ok(sessions)
}

fn parse_session_file(path: &Path, max_turns: usize) -> Result<Option<SessionRecord>> {
    let content = fs::read_to_string(path).with_context(|| format!("read failed for {}", path.display()))?;
    let mut session_id = String::new();
    let mut cwd: Option<PathBuf> = None;
    let mut turns = Vec::new();

    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let row: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if session_id.is_empty() {
            session_id = row.get("sessionId").and_then(Value::as_str).unwrap_or_default().to_string();
        }
        if cwd.is_none() {
            cwd = row
                .get("cwd")
                .and_then(Value::as_str)
                .map(PathBuf::from);
        }
        let row_type = row.get("type").and_then(Value::as_str).unwrap_or_default();
        if row_type == "user" || row_type == "assistant" {
            let raw_text = extract_text(
                row.get("message")
                    .and_then(|m| m.get("content"))
                    .unwrap_or(&Value::Null),
            );
            let text = clean_low_entropy_text(&raw_text);
            if !text.is_empty() {
                turns.push(SessionTurn {
                    role: row_type.to_string(),
                    text,
                    timestamp: row
                        .get("timestamp")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                });
            }
        }
    }
    if session_id.is_empty() || cwd.is_none() {
        return Ok(None);
    }
    if turns.len() > max_turns {
        turns = turns.split_off(turns.len() - max_turns);
    }

    let metadata = fs::metadata(path)?;
    let modified = metadata.modified()?;
    let updated_at = DateTime::<Utc>::from(modified);
    Ok(Some(SessionRecord {
        session_id,
        file_path: path.to_path_buf(),
        project_cwd: cwd.unwrap_or_default(),
        updated_at,
        turns,
    }))
}

fn extract_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.trim().to_string(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                if item.get("type").and_then(Value::as_str) == Some("text") {
                    item.get("text").and_then(Value::as_str).map(str::trim).map(ToString::to_string)
                } else {
                    None
                }
            })
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn clean_low_entropy_text(raw: &str) -> String {
    let cleaned = raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !is_low_entropy_line(line))
        .collect::<Vec<_>>();
    cleaned.join("\n")
}

fn is_low_entropy_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    let starts = [
        "exit code:",
        "command output:",
        "elapsed_ms:",
        "running_for_ms:",
        "started_at:",
        "ended_at:",
        "output_file_path:",
        "cwd:",
        "pid:",
        "total ",
        "drwx",
        "-rw",
        "l1:",
        "l2:",
        "l3:",
        "---",
    ];
    if starts.iter().any(|prefix| lower.starts_with(prefix)) {
        return true;
    }
    let contains = [
        "shell state (cwd, env vars)",
        "command completed in",
        "downloaded ",
        "compiling ",
        "finished `dev` profile",
        "running `/home/",
        "rows x",
        "columns",
        ".parquet",
        ".csv",
        ".jsonl",
    ];
    contains.iter().any(|needle| lower.contains(needle))
}

fn turns_as_text(session: &SessionRecord) -> String {
    session
        .turns
        .iter()
        .map(|t| format!("[{}] {}", t.role, t.text))
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_under_watch(path: &Path, watch_roots: &[PathBuf]) -> bool {
    let Some(candidate) = canonicalize_or_none(path) else {
        return false;
    };
    watch_roots.iter().any(|root| {
        canonicalize_or_none(root)
            .map(|canonical_root| candidate.starts_with(canonical_root))
            .unwrap_or(false)
    })
}

fn canonicalize_or_none(path: &Path) -> Option<PathBuf> {
    fs::canonicalize(path).ok()
}

fn write_default_config(path: &Path) -> Result<()> {
    let cfg = AppConfig::default();
    write_json_file(path, &cfg)
}

fn load_config(path: &Path) -> Result<AppConfig> {
    if !path.exists() {
        return Ok(AppConfig::default());
    }
    let content = fs::read_to_string(path)?;
    let value: Value = serde_json::from_str(&content)?;
    let mut cfg = AppConfig::default();
    if let Some(v) = value.get("sessions_root").and_then(Value::as_str) {
        cfg.sessions_root = v.to_string();
    }
    if let Some(v) = value.get("watch_roots").and_then(Value::as_array) {
        cfg.watch_roots = v
            .iter()
            .filter_map(Value::as_str)
            .map(ToString::to_string)
            .collect();
    }
    if let Some(v) = value.get("max_sessions").and_then(Value::as_u64) {
        cfg.max_sessions = v as usize;
    }
    if let Some(v) = value.get("max_turns_per_session").and_then(Value::as_u64) {
        cfg.max_turns_per_session = v as usize;
    }
    if let Some(v) = value.get("aggregate_title").and_then(Value::as_str) {
        cfg.aggregate_title = v.to_string();
    }
    if let Some(v) = value.get("memory_catalog_path").and_then(Value::as_str) {
        cfg.memory_catalog_path = v.to_string();
    }
    if let Some(v) = value.get("station_root").and_then(Value::as_str) {
        cfg.station_root = v.to_string();
    }
    Ok(cfg)
}

fn expand_path(raw: &str) -> PathBuf {
    if let Some(stripped) = raw.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(stripped);
        }
    }
    PathBuf::from(raw)
}

fn write_text_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

fn write_json_file<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let body = serde_json::to_string_pretty(value)? + "\n";
    write_text_file(path, &body)
}

fn load_catalog(path: &Path) -> Result<MemoryCatalog> {
    if !path.exists() {
        return Ok(MemoryCatalog::default());
    }
    let body = fs::read_to_string(path)?;
    let parsed = serde_json::from_str(&body)?;
    Ok(parsed)
}

fn save_catalog(path: &Path, catalog: &MemoryCatalog) -> Result<()> {
    write_json_file(path, catalog)
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let truncated = s.chars().take(max_chars).collect::<String>();
    format!("{truncated}\n\n...[truncated]...")
}

fn topic_slug_from_markdown(markdown: &str) -> String {
    let lower = markdown.to_ascii_lowercase();

    let strategy_research = [
        "hypothesis",
        "signal",
        "walk-forward",
        "backtest",
        "baseline",
        "edge",
        "kelly",
        "regime",
        "distribution",
        "sortino",
    ];
    let execution_trading = [
        "executionbackend",
        "sessionrunner",
        "liveexecutionbackend",
        "place_order",
        "nautilus",
        "fill",
        "order",
        "drawdown",
        "trade",
    ];
    let data_ingestion_pipeline = [
        "tradestation",
        "stream_bars",
        "live-data-ingestion",
        "jsonl",
        "parquet",
        "watchlist",
        "bars",
        "backfill",
        "rate limit",
    ];
    let platform_ops = [
        "docker",
        "deploy",
        "release",
        "ci",
        "build",
        "runtime",
        "staging",
        "crucible",
        "auth token",
        "oauth",
    ];
    let coordination_docs = [
        "pr #",
        "issue #",
        "roadmap",
        "handoff",
        "documentation",
        "talk",
        "template",
        "coordination",
        "cross-team",
    ];

    // Coarse taxonomy only: avoid brittle micro-topics.
    let taxonomy: [(&str, &[&str]); 5] = [
        ("strategy-research", &strategy_research),
        ("execution-trading", &execution_trading),
        ("data-ingestion-pipeline", &data_ingestion_pipeline),
        ("platform-ops", &platform_ops),
        ("coordination-docs", &coordination_docs),
    ];

    let mut best = ("mixed-topics", 0usize);
    for (slug, keys) in taxonomy {
        let score = keys.iter().filter(|k| lower.contains(**k)).count();
        if score > best.1 {
            best = (slug, score);
        }
    }
    best.0.to_string()
}

fn remove_stale_topic_copies(topics_dir: &Path, session_id: &str) -> Result<()> {
    if !topics_dir.exists() {
        return Ok(());
    }
    let session_filename = format!("{session_id}.md");
    for entry in fs::read_dir(topics_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let old_copy = path.join(&session_filename);
        if old_copy.exists() {
            fs::remove_file(&old_copy).with_context(|| format!("failed removing stale copy {}", old_copy.display()))?;
        }
    }
    Ok(())
}

fn horizon_lane(horizon: Horizon) -> &'static str {
    match horizon {
        Horizon::ShortTerm => "short-term",
        Horizon::LongTerm => "long-term",
    }
}

fn split_tags(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn slugify(raw: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in raw.chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            ch.to_ascii_lowercase()
        } else {
            '-'
        };
        if mapped == '-' {
            if !prev_dash {
                out.push('-');
                prev_dash = true;
            }
        } else {
            out.push(mapped);
            prev_dash = false;
        }
    }
    out.trim_matches('-').to_string()
}
