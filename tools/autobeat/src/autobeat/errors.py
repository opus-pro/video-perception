"""Domain errors for auto-beat-grid CLI."""


class AutobeatError(RuntimeError):
    """Raised when beat-grid analysis fails."""


class TrackLayoutError(RuntimeError):
    """Raised when span-track layout resolution fails."""
