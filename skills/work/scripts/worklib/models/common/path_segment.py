"""Cross-platform path segment policy model."""


class PathSegmentPolicy:
    WINDOWS_DEVICES = {
        "CON", "PRN", "AUX", "NUL",
        *(f"COM{number}" for number in range(1, 10)),
        *(f"LPT{number}" for number in range(1, 10)),
    }
    UNSAFE_CHARACTERS = frozenset('<>:"|?*')

    @classmethod
    def issue(cls, segment: str) -> str | None:
        if not segment or segment in {".", ".."}:
            return "unsafe_path_segment"
        if segment[-1] in {" ", "."}:
            return "unsafe_path_segment"
        if any(ord(character) < 32 for character in segment) or any(
            character in cls.UNSAFE_CHARACTERS for character in segment
        ):
            return "unsafe_path_segment"
        if segment.split(".", 1)[0].upper() in cls.WINDOWS_DEVICES:
            return "windows_device_name"
        return None


__all__ = ["PathSegmentPolicy"]
