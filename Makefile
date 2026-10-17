# video-perception Makefile

.PHONY: setup build install-tools pull-model doctor check test test-cli test-tools clean

# --- Setup (full onboarding) ---

setup: build install-tools doctor
	@echo ""
	@echo "video-perception ready. Run: video-perception catalog list"

# --- Build ---

build:
	cd cli && cargo build --release
	@echo "Binary: cli/target/release/video-perception"

install-tools:
	@echo "Installing Python tools via uv..."
	uv tool install --python 3.11 tools/autobeat
	uv tool install --python 3.11 tools/spectral
	uv tool install --python 3.11 tools/audio-class
	uv tool install --python 3.11 tools/vlm
	uv tool install --python 3.11 tools/motion-snn
	@echo "Tools installed."

# --- Models (opt-in, ~9.6 GB) ---

pull-model:
	ollama pull gemma4

# --- Check / Doctor ---

check:
	@command -v ffmpeg >/dev/null 2>&1 || (echo "MISSING: ffmpeg" && exit 1)
	@command -v ffprobe >/dev/null 2>&1 || (echo "MISSING: ffprobe" && exit 1)
	@command -v magick >/dev/null 2>&1 || echo "OPTIONAL: imagemagick not found"
	@echo "Core deps OK."

doctor:
	cli/target/release/video-perception doctor

# --- Test ---

test: test-cli test-tools
	@echo "All tests passed."

test-cli:
	cd cli && cargo test

test-tools:
	cd tools/autobeat && uv run --isolated --no-project --with pytest --with . --python 3.11 python -m pytest tests/ -q
	cd tools/motion-snn && uv run --isolated --no-project --with pytest --with . --python 3.11 python -m pytest tests/ -q
	cd tools/vlm && uv run --isolated --no-project --with pytest --with . --python 3.11 python -m pytest tests/ -q

# --- Clean ---

clean:
	uv tool uninstall autobeat 2>/dev/null || true
	uv tool uninstall video-perception-spectral 2>/dev/null || true
	uv tool uninstall video-perception-audio-class 2>/dev/null || true
	uv tool uninstall video-perception-vlm 2>/dev/null || true
	uv tool uninstall video-perception-motion-snn 2>/dev/null || true
	find tools -name "__pycache__" -type d -exec rm -rf {} + 2>/dev/null || true
	rm -rf obs/
