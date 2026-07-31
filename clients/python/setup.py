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
PROTOCOL_ROOT = PACKAGE_ROOT.parent.parent / "protocol"
PROTOCOL_GENERATOR = PROTOCOL_ROOT / "generate.ts"
GENERATED_PYTHON_FILES = (
    PACKAGE_ROOT / "src" / "openkache" / "_generated" / "smithy_api.py",
    PACKAGE_ROOT / "src" / "openkache" / "_generated" / "smithy_contract.py",
)


def native_library_name() -> str:
    if os.name == "nt":
        return "openkache_client_python_native.dll"
    if sys.platform == "darwin":
        return "libopenkache_client_python_native.dylib"
    return "libopenkache_client_python_native.so"


def generate_smithy_contract() -> None:
    """Generate Python contracts from the canonical Smithy model."""

    if not PROTOCOL_GENERATOR.is_file():
        if all(path.is_file() for path in GENERATED_PYTHON_FILES):
            # Source distributions carry the already generated contract but
            # intentionally do not carry the private protocol generator tree.
            return
        raise RuntimeError(
            "Python package builds require protocol/generate.ts and its Smithy "
            "model, or generated _generated/smithy_*.py files from a source "
            "distribution."
        )
    environment = os.environ.copy()
    environment["OPENKACHE_GENERATION_TARGET"] = "python"
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
        bundled = PACKAGE_ROOT / "src" / "openkache" / "_native.so"
        destination = Path(self.build_lib) / "openkache" / "_native.so"
        destination.parent.mkdir(parents=True, exist_ok=True)
        if bundled.is_file():
            shutil.copy2(bundled, destination)
            return
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
    """Include the host native artifact so wheel builds from sdist stay hermetic."""

    def run(self) -> None:
        self.run_command("generate_smithy")
        self.run_command("build_native")
        super().run()

    def make_release_tree(self, base_dir: str, files: list[str]) -> None:
        super().make_release_tree(base_dir, files)
        source = NATIVE_ROOT / "target" / "release" / native_library_name()
        if not source.is_file():
            raise RuntimeError(f"Cargo did not produce {source}")
        destination = Path(base_dir) / "src" / "openkache" / "_native.so"
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)


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
