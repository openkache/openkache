"""Build the wheel's portable ctypes Rust adapter with Cargo."""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path

from setuptools import Command, setup
from setuptools.command.build_py import build_py as _build_py
from setuptools.command.sdist import sdist as _sdist
from wheel.bdist_wheel import bdist_wheel as _bdist_wheel


PACKAGE_ROOT = Path(__file__).resolve().parent
NATIVE_ROOT = PACKAGE_ROOT / "native"
PUBLIC_ROOT = PACKAGE_ROOT.parent.parent
PROTOCOL_ROOT = PUBLIC_ROOT / "protocol"
if not PROTOCOL_ROOT.is_dir():
    PROTOCOL_ROOT = PACKAGE_ROOT / "protocol"
CORE_ROOT = PUBLIC_ROOT / "clients" / "core"
if not CORE_ROOT.is_dir():
    CORE_ROOT = PACKAGE_ROOT / "core"
PROTOCOL_GENERATOR = PROTOCOL_ROOT / "generate.ts"


def native_library_name() -> str:
    if os.name == "nt":
        return "openkache_client_python_native.dll"
    if sys.platform == "darwin":
        return "libopenkache_client_python_native.dylib"
    return "libopenkache_client_python_native.so"


def generate_smithy_contract() -> None:
    """Generate Python contracts from the canonical Smithy model."""

    if not PROTOCOL_GENERATOR.is_file():
        raise RuntimeError(
            "Python package builds require the bundled protocol/generate.ts "
            "and Smithy model."
        )
    environment = os.environ.copy()
    environment["OPENKACHE_GENERATION_TARGET"] = "python"
    generated_root = PACKAGE_ROOT / "src" / "openkache" / "_generated"
    environment["OPENKACHE_PYTHON_API_OUTPUT"] = str(generated_root / "smithy_api.py")
    environment["OPENKACHE_PYTHON_CONTRACT_OUTPUT"] = str(
        generated_root / "smithy_contract.py"
    )
    try:
        subprocess.run(
            [
                os.environ.get("BUN", "bun"),
                str(PROTOCOL_GENERATOR),
            ],
            cwd=PROTOCOL_ROOT,
            env=environment,
            check=True,
        )
    except FileNotFoundError as error:
        raise RuntimeError(
            "Python package builds require Bun and the Smithy CLI; "
            "install both before running `python -m build`."
        ) from error


class generate_smithy(Command):
    """Generate the Python Smithy API and contract modules."""

    description = "generate Python Smithy contract modules"
    user_options: list[tuple[str, str | None, str]] = []

    def initialize_options(self) -> None:
        pass

    def finalize_options(self) -> None:
        pass

    def run(self) -> None:
        generate_smithy_contract()


class build_native(Command):
    """Compile and copy the shared client-core ABI into the Python package."""

    description = "build the Rust client-core ctypes adapter"
    user_options: list[tuple[str, str | None, str]] = []

    def initialize_options(self) -> None:
        self.build_lib: str | None = None

    def finalize_options(self) -> None:
        build_py = self.get_finalized_command("build_py")
        self.build_lib = build_py.build_lib

    def run(self) -> None:
        if self.build_lib is None:
            raise RuntimeError("setuptools did not initialize build_lib")
        destination = Path(self.build_lib) / "openkache" / native_library_name()
        destination.parent.mkdir(parents=True, exist_ok=True)
        legacy_destination = destination.with_name("_native.so")
        if legacy_destination.is_file():
            legacy_destination.unlink()
        environment = os.environ.copy()
        environment.pop("CARGO_BUILD_TARGET", None)
        environment["CARGO_TARGET_DIR"] = str(NATIVE_ROOT / "target")
        subprocess.run(
            [
                os.environ.get("CARGO", "cargo"),
                "build",
                "--locked",
                "--release",
                "--manifest-path",
                str(NATIVE_ROOT / "Cargo.toml"),
            ],
            cwd=PACKAGE_ROOT,
            env=environment,
            check=True,
        )
        native_name = native_library_name()
        source = NATIVE_ROOT / "target" / "release" / native_name
        if not source.is_file():
            raise RuntimeError(f"Cargo did not produce {source}")
        shutil.copy2(source, destination)


class build_py(_build_py):
    """Run the native build after Python sources have been staged."""

    def run(self) -> None:
        self.run_command("generate_smithy")
        super().run()
        self.run_command("build_native")


class sdist(_sdist):
    """Bundle shared Rust sources so source builds compile native code per platform."""

    def run(self) -> None:
        self.run_command("generate_smithy")
        super().run()

    def make_release_tree(self, base_dir: str, files: list[str]) -> None:
        super().make_release_tree(base_dir, files)
        release_root = Path(base_dir)
        source_ignore = shutil.ignore_patterns(
            "__pycache__",
            "*.pyc",
            "generated_local",
            "target",
        )
        shutil.copytree(CORE_ROOT, release_root / "core", ignore=source_ignore)
        shutil.copytree(PROTOCOL_ROOT, release_root / "protocol", ignore=source_ignore)
        _replace_path_dependency(
            release_root / "native" / "Cargo.toml",
            'path = "../../core"',
            'path = "../core"',
        )
        _replace_path_dependency(
            release_root / "core" / "Cargo.toml",
            'path = "../../protocol"',
            'path = "../protocol"',
        )


def _replace_path_dependency(path: Path, source: str, destination: str) -> None:
    content = path.read_text(encoding="utf-8")
    if content.count(source) != 1:
        raise RuntimeError(
            f"expected exactly one {source!r} dependency path in {path}"
        )
    path.write_text(content.replace(source, destination), encoding="utf-8")


class bdist_wheel(_bdist_wheel):
    """Mark ctypes wheels as platform-specific without a Python ABI lock."""

    def finalize_options(self) -> None:
        super().finalize_options()
        self.root_is_pure = False

    def get_tag(self) -> tuple[str, str, str]:
        _, _, platform = super().get_tag()
        return "py3", "none", platform


setup(
    cmdclass={
        "generate_smithy": generate_smithy,
        "build_native": build_native,
        "build_py": build_py,
        "bdist_wheel": bdist_wheel,
        "sdist": sdist,
    }
)
