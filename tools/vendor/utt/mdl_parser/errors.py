class PSGError(Exception):
    """Base class for PSG parsing failures."""


class PSGFormatError(PSGError):
    """Raised when a file is not a supported EA Skate PS3 model PSG."""


class PSGDataError(PSGError):
    """Raised when a supported PSG contains invalid or truncated data."""
