# MCP Server

> Feature 55 · P2 · GitHub issue [#55](https://github.com/yicheng47/pulse/issues/55). Agents cannot see or manage the Pulse library; an MCP server over stdio exposes library reads, playlist CRUD, and library maintenance, following the Quill pattern.

## Motivation

The library lives in SQLite behind the GPUI app, so the only client is the app itself. An MCP server makes the library agent-operable: ask what's in the library, build and curate playlists from a conversation, trigger maintenance — the same workflow Quill already proves out (its app binary serves `mcp` over stdio; read tools are always available, write tools are gated by an in-app setting).

## Scope

- **`pulse mcp` stdio entry point** on the app binary: the minimal MCP subset (initialize, tools/list, tools/call) as hand-rolled stdio JSON-RPC — the protocol surface is small; no framework dependency unless phase 1 proves otherwise.
- **Library read tools**, always available: list/search albums, tracks, and artists (title/artist/genre filters), track detail including format metadata (codec, bit depth, sample rate, duration), list playlists with contents, list storage roots, library stats.
- **Playlist write tools**, gated: create, rename, delete; add, remove, and reorder tracks.
- **Library maintenance tools**, gated: trigger a rescan of storage roots; report tracks whose files are missing.
- **Write gating**: a Settings toggle ("Allow MCP write access", default off) mirroring Quill; write tools exist in tools/list but refuse with a clear message while disabled.
- **Cross-process safety**: the library store opens in WAL mode with a busy timeout; the app picks up external changes when its window activates.
- **Same data-dir resolution as the app** (feature 04): debug builds serve `pulse-dev`, release builds serve `pulse`.
- Registration documented for Claude Code: `claude mcp add pulse -- /Applications/Pulse.app/Contents/MacOS/pulse mcp`.

## Non-Goals

- Playback control from agents (play/pause/queue) — needs IPC into the running app, a Runner-style proxy; separate feature if wanted.
- Editing file tags or moving/deleting audio files on disk.
- Adding or removing storage roots via MCP — library shape stays a human decision in-app.
- Network transports, remote access, or auth — stdio only, local machine only.
- Cover-art bytes over the wire; tools may return paths.

## Implementation Phases

1. Extract the library store (album/track/playlist/storage-root queries) from `library_ui` into a UI-free module usable headlessly; enable WAL + busy timeout.
2. `pulse mcp` entry: stdio JSON-RPC loop, initialize/tools-list/tools-call dispatch, read tools against the extracted store.
3. Write tools plus the Settings gate; app-side refresh on window activate so external edits appear without relaunch.
4. Registration docs; validation against a real agent session.

## Verification

- Unit tests: tool dispatch against a fixture library DB — read tools return correct shapes; write tools refuse when the gate is off and mutate correctly when on; a second connection writing mid-session does not corrupt or deadlock (WAL).
- `make verify` is green.
- Manual: register the debug binary in Claude Code; an agent lists albums, creates a playlist, adds tracks by search, reorders, deletes — each step visible in the app after window activate; the toggle off blocks every write with a clear refusal; the debug build touches only `pulse-dev`.
