# video-perception (npm launcher)

Runs the [Video Perception](https://github.com/opus-pro/video-perception) CLI
and MCP server without a separate install step:

```bash
npx video-perception capabilities --json
npx video-perception mcp
```

The first run downloads the release binary for this machine from the GitHub
release that matches the package version, verifies it against the release
`checksums.txt`, and caches it under `$XDG_CACHE_HOME/video-perception` (or
`~/.cache/video-perception`). Later runs start the cached binary directly. The
launcher writes only to stderr, so `mcp` keeps stdout clean.

Published binaries cover macOS (Apple Silicon and Intel) and Linux x86_64
(glibc and musl). Other platforms build from source. FFmpeg and ffprobe must be
on `PATH`; ImageMagick enables contact sheets.

Set `VIDEO_PERCEPTION_NPM_CACHE` to change the cache directory, or
`VIDEO_PERCEPTION_RELEASE_BASE_URL` to download from a mirror that serves the
same asset names and `checksums.txt`.
