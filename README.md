# Context Radar

`context-radar` is a Rust CLI that monitors Claude Code session logs across watched folders and produces high-signal context artifacts with Haiku.

## Why

When you run many Claude sessions across repos, context gets fragmented. This tool builds a single "what happened / what next" report from local `~/.claude/projects/*/*.jsonl` logs.

## Core capabilities

- Watches all Claude session logs under a configurable sessions root.
- Filters sessions to projects under configured watch roots.
- `summarize`: per-session summaries + aggregate digest.
- `author-context`: topic disentangling + high-entropy extraction + authored context window.
- Low-entropy cleanup at ingest (filters shell output/file-read noise).
- `curate`: persist a curated context in a memory catalog with `short_term` / `long_term` horizon.
- `kickoff`: assemble selected curated contexts into a non-interactive kickoff packet.
- `station-add`: save repo-scoped curated memory in Tokyo lanes (`short-term` / `long-term`).
- `station-monthly`: create monthly knowledge deep dives per repo/lane using Haiku.

## Install

```bash
cd /home/pt/projects/context-radar
cargo build --release
```

## Quick start

```bash
context-radar init-config
context-radar scan
context-radar summarize
context-radar author-context
context-radar curate --title "Spec-121 split state" --horizon long-term --context-file reports/latest-authored-context-window.md --tags "spec121,split"
context-radar kickoff --horizon both
context-radar station-add --repo cannopy --horizon long-term --title "Spec 121 decisions" --summary-file reports/latest-authored-context-window.md --tags "spec121,data-plane"
context-radar station-monthly --repo cannopy --month 2026-04 --horizon long-term
```

Default outputs:

- `reports/latest-session-digest.md`
- `reports/latest-session-summaries.json`
- `reports/latest-authored-context-window.md`
- `reports/latest-authored-context-window.json`
- `reports/kickoff-context.md`

## Config

Create or edit `context-radar.config.json`:

```json
{
  "sessions_root": "/home/pt/.claude/projects",
  "watch_roots": [
    "/home/pt/projects",
    "/mnt/c/Users/pabto/projects"
  ],
  "max_sessions": 12,
  "max_turns_per_session": 8,
  "aggregate_title": "Claude Code Session Digest",
  "memory_catalog_path": "data/memory/catalog.json",
  "station_root": "data/stations"
}
```

## Storage model (current)

- **Operational metadata:** JSON catalog at `data/memory/catalog.json` (curated context index).
- **Context artifacts:** markdown/json in `reports/`.
- **Repo memory lanes:** `data/stations/<repo>/{short-term,long-term}/`.
- **Design intent:** local-first, Rust-native stack; Claude/Haiku does semantic heavy lifting.

## Open-source roadmap

- Rust-native index backend option (`redb`/`sled`) if JSON catalog reaches scale limits.
- Optional S3 archival for enterprise long-term audit retention (roadmap item).
- Optional Parquet exports for structured interchange snapshots (not primary memory store).
- Topic graph across sessions and context-window quality scoring.

