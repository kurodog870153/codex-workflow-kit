from __future__ import annotations

import ast
import tempfile
import unittest
from dataclasses import dataclass
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]
WORKLIB_ROOT = PROJECT_ROOT / "skills" / "work" / "scripts" / "worklib"
LEGACY_ROOTS = {"artifacts", "contracts", "execution"}
LEGACY_SERVICE_EXCEPTIONS = {
    ("worklib.services.progress.validation", "worklib.contracts.validation"),
}


@dataclass(frozen=True)
class ImportEdge:
    source: str
    target: str
    path: Path
    line: int


def _module_name(root: Path, path: Path) -> str:
    relative = path.relative_to(root).with_suffix("")
    parts = list(relative.parts)
    if parts[-1] == "__init__":
        parts.pop()
    return ".".join(["worklib", *parts])


def _relative_target(
    source: str,
    level: int,
    module: str | None,
    *,
    is_package: bool,
) -> str:
    package = source if is_package else source.rpartition(".")[0]
    parts = package.split(".") if package else []
    if level > 1:
        parts = parts[: -(level - 1)]
    if module:
        parts.extend(module.split("."))
    return ".".join(parts)


def _import_edges(root: Path, path: Path) -> list[ImportEdge]:
    source = _module_name(root, path)
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    result: list[ImportEdge] = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for alias in node.names:
                result.append(ImportEdge(source, alias.name, path, node.lineno))
        elif isinstance(node, ast.ImportFrom):
            if node.level:
                target = _relative_target(
                    source,
                    node.level,
                    node.module,
                    is_package=path.name == "__init__.py",
                )
            else:
                target = node.module or ""
            if node.module is None:
                for alias in node.names:
                    if alias.name != "*":
                        result.append(
                            ImportEdge(
                                source,
                                ".".join(part for part in (target, alias.name) if part),
                                path,
                                node.lineno,
                            )
                        )
            else:
                result.append(ImportEdge(source, target, path, node.lineno))
    return result


def _parts(module: str) -> tuple[str, ...]:
    parts = tuple(module.split("."))
    return parts[1:] if parts and parts[0] == "worklib" else parts


def _source_scope(root: Path, path: Path, *, include_controllers: bool) -> tuple[str, str | None] | None:
    relative = path.relative_to(root)
    parts = relative.parts
    if not parts:
        return None
    if parts[0] == "controllers" and include_controllers:
        name = parts[1].removesuffix(".py") if len(parts) > 1 else None
        return "controller", name
    if parts[0] == "business_services":
        name = parts[1].removesuffix(".py") if len(parts) > 1 else None
        return "business_service", name
    if parts[0] == "services" and len(parts) >= 3:
        return "service", parts[1]
    if parts[0] == "models":
        name = parts[1].removesuffix(".py") if len(parts) > 1 else None
        return "model", name
    if parts[0] == "protocol":
        return "protocol", None
    return None


def _target_scope(module: str) -> tuple[str, str | None] | None:
    parts = _parts(module)
    if not parts:
        return None
    if parts[0] == "controllers":
        return "controller", parts[1] if len(parts) > 1 else None
    if parts[0] == "business_services":
        return "business_service", parts[1] if len(parts) > 1 else None
    if parts[0] == "services":
        return "service", parts[1] if len(parts) > 1 else None
    if parts[0] == "models":
        return "model", parts[1] if len(parts) > 1 else None
    if parts[0] == "protocol":
        return "protocol", None
    if parts[0] in {"foundation", "infrastructure"}:
        return parts[0], parts[1] if len(parts) > 1 else None
    if parts[0] in LEGACY_ROOTS:
        return "legacy", parts[0]
    return None


def _import_violation(
    source_scope: tuple[str, str | None],
    target_scope: tuple[str, str | None],
) -> str | None:
    source_layer, source_name = source_scope
    target_layer, target_name = target_scope
    if source_layer == "protocol":
        if target_layer == "protocol":
            return None
        return "protocol may not depend on a product layer"
    if target_layer == "protocol":
        return None
    if source_layer == "controller":
        if target_layer == "controller":
            return None
        if target_layer == "business_service" and target_name == source_name:
            return None
        return "controller may only import its matching business service"
    if source_layer == "business_service":
        if target_layer in {"service", "model"}:
            return None
        if target_layer == "business_service" and target_name == source_name:
            return None
        return "business service has a forbidden dependency"
    if source_layer == "service":
        if target_layer in {"model", "foundation", "infrastructure"}:
            return None
        if target_layer == "service" and target_name == source_name:
            return None
        return "single-function service may not import another service"
    if source_layer == "model" and target_layer != "model":
        return "model may only import another model"
    return None


def architecture_violations(root: Path, *, include_controllers: bool = False) -> list[str]:
    result: list[str] = []
    for path in sorted(root.rglob("*.py")):
        source_scope = _source_scope(root, path, include_controllers=include_controllers)
        if source_scope is None:
            continue
        source = _module_name(root, path)
        if source_scope[0] == "model":
            tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
            for node in tree.body:
                if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                    result.append(
                        f"{path.relative_to(root)}:{node.lineno}: "
                        "model module may not define module-level functions"
                    )
        for edge in _import_edges(root, path):
            if not edge.target.startswith("worklib."):
                continue
            target_scope = _target_scope(edge.target)
            if target_scope is None:
                continue
            if (edge.source, edge.target) in LEGACY_SERVICE_EXCEPTIONS:
                continue
            reason = _import_violation(source_scope, target_scope)
            if reason:
                result.append(
                    f"{edge.path.relative_to(root)}:{edge.line}: {edge.source} -> "
                    f"{edge.target}: {reason}"
                )
    return result


class ArchitectureBoundaryTests(unittest.TestCase):
    def write(self, root: Path, relative: str, text: str = "") -> None:
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def test_current_new_architecture_packages_follow_boundaries(self) -> None:
        self.assertEqual(architecture_violations(WORKLIB_ROOT), [])

    def test_valid_four_layer_slice_is_accepted(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.write(
                root,
                "controllers/progress.py",
                "from worklib.business_services.progress import run\n",
            )
            self.write(
                root,
                "business_services/progress/__init__.py",
                "from worklib.models.progress import Progress\n"
                "from worklib.services.progress_read import read_progress\n",
            )
            self.write(
                root,
                "services/progress_read/__init__.py",
                "from worklib.foundation.jsonio import canonical_json\n"
                "from worklib.infrastructure.progress_storage import read_progress\n"
                "from worklib.models.progress import Progress\n",
            )
            self.write(root, "models/progress.py", "class Progress:\n    pass\n")
            self.write(
                root,
                "protocol/shared.py",
                'SHA256_PATTERN = r"^[0-9a-f]{64}$"\n',
            )
            self.assertEqual(
                architecture_violations(root, include_controllers=True),
                [],
            )

    def test_controller_cannot_skip_business_service(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.write(
                root,
                "controllers/progress.py",
                "from worklib.services.progress_read import read_progress\n",
            )
            violations = architecture_violations(root, include_controllers=True)
            self.assertEqual(len(violations), 1)
            self.assertIn("controller may only import", violations[0])

    def test_model_cannot_depend_upward_or_define_function(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.write(
                root,
                "models/progress.py",
                "from worklib.services.progress_read import read_progress\n\n"
                "def load():\n    return read_progress()\n",
            )
            violations = architecture_violations(root)
            self.assertEqual(len(violations), 2)
            self.assertTrue(any("module-level functions" in item for item in violations))
            self.assertTrue(any("model may only import" in item for item in violations))

    def test_model_may_import_another_model(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.write(
                root,
                "models/common/cli.py",
                "from worklib.models.common.base import WorkContract\n",
            )
            self.write(root, "models/common/base.py", "class WorkContract:\n    pass\n")
            self.assertEqual(architecture_violations(root), [])


    def test_service_relative_reexport_cannot_hide_cross_feature_import(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.write(
                root,
                "services/alpha/__init__.py",
                "from ..beta import run\n",
            )
            self.write(root, "services/beta/__init__.py", "def run():\n    pass\n")
            violations = architecture_violations(root)
            self.assertEqual(len(violations), 1)
            self.assertIn("worklib.services.alpha -> worklib.services.beta", violations[0])

    def test_only_recorded_progress_validation_legacy_import_is_allowed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.write(root, "services/progress/validation.py", "from worklib.contracts.validation import strict_keys\n")
            self.write(root, "services/progress/read.py", "from worklib.contracts.validation import strict_keys\n")
            violations = architecture_violations(root)
            self.assertEqual(len(violations), 1)
            self.assertIn("worklib.services.progress.read", violations[0])

    def test_business_services_cannot_import_each_other(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.write(
                root,
                "business_services/plan/__init__.py",
                "from worklib.business_services.task import create_task\n",
            )
            self.write(
                root,
                "business_services/task/__init__.py",
                "def create_task():\n    pass\n",
            )
            violations = architecture_violations(root)
            self.assertEqual(len(violations), 1)
            self.assertIn("business service has a forbidden dependency", violations[0])

    def test_protocol_cannot_import_product_layer(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.write(
                root,
                "protocol/shared.py",
                "from worklib.models.progress import Progress\n",
            )
            violations = architecture_violations(root)
            self.assertEqual(len(violations), 1)
            self.assertIn("protocol may not depend on a product layer", violations[0])


if __name__ == "__main__":
    unittest.main()
