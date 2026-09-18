from __future__ import annotations

from typing import Any, Callable

from ....models.common.errors import WorkError


class Diagnostics:
    def __init__(self):
        self.checks: list[dict[str, Any]] = []
        self.issues: list[dict[str, Any]] = []

    def skip(self, name: str, *dependencies: str):
        self.checks.append({"name": name, "status": "not_checked", "requires": list(dependencies)})
        return None

    def check(self, name: str, operation: Callable, *, location: str = ""):
        try:
            value = operation()
        except WorkError as error:
            self.failure(name, error, location=location)
            return None
        except (TypeError, KeyError, ValueError, RecursionError) as error:
            self.skip(name, "supported_input_shape")
            self.issues.append({
                "stage": name, "code": "diagnostic_input_not_supported", "location": location,
                "category": "review_required",
                "message": "This check cannot continue with the supplied value.",
                "suggestion": "Review the input shape, then repeat diagnosis.",
                "details": {"exception_type": type(error).__name__},
            })
            return None
        self.checks.append({"name": name, "status": "passed"})
        return value

    def failure(self, name: str, error: WorkError, *, location: str = ""):
        self.checks.append({"name": name, "status": "failed"})
        category = "review_required"
        suggestion = "Review the evidence before preparing a repair; do not change source fingerprints alone."
        leaf = name.rsplit(":", 1)[-1]
        if leaf in {"encoding", "json"}:
            category = "user_decision"
            suggestion = "Preserve the original bytes; ask the user to resolve any ambiguous decoding or JSON interpretation."
        elif leaf in {"normalization", "canonical"}:
            category = "format_repair"
            suggestion = "Preview a lossless canonical UTF-8 rendering before requesting write approval."
        elif name.startswith("instructions"):
            category = "source_review"
        self.issues.append({
            "stage": name, "code": error.code, "location": location,
            "category": category, "message": error.message,
            "suggestion": suggestion, "details": dict(error.details),
        })

    def passed(self, name: str) -> bool:
        return any(check["name"] == name and check["status"] == "passed" for check in self.checks)

    def status(self, name: str) -> str:
        return next((check["status"] for check in self.checks if check["name"] == name), "not_checked")


__all__ = ["Diagnostics"]


