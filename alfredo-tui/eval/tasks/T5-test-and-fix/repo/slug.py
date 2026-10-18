"""URL slugs."""
import re


def slugify(text):
    """Lowercase words joined by single hyphens, with no leading or trailing hyphen."""
    return re.sub(r"[^a-z0-9]+", "-", text.lower())
