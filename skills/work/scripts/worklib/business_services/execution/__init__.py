"""Execution lifecycle business service with lazy public exports."""


def __getattr__(name: str):
    if name not in __all__:
        raise AttributeError(name)
    from .workflow import ExecutionService
    return ExecutionService

__all__ = ["ExecutionService"]
