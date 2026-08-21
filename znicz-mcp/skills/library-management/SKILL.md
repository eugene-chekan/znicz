---
name: library-management
description: Scan, index, and search local music libraries in znicz (Phase 2). Use when organizing files, browsing metadata, or building search queries.
---

# Library Management

## Status

Phase 2 — tools `search_library`, `get_track`, and `browse_album` are stubbed until the library DB ships.

## Planned workflow

1. Configure `[library].paths` in config
2. Scan with lofty metadata extraction
3. Store in rusqlite with FTS search
4. Use MCP library tools and `znicz://library/*` resources
