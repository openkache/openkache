"""Build the wheel's portable ctypes Rust adapter.

Repository checkouts provide a Bazel-built native artifact. Source
distributions remain self-contained and compile their adapter with Cargo when
they are built outside the repository.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from setuptools import Command, setup
from setuptools.command.bdist_wheel import bdist_wheel as _bdist_wheel
from setuptools.command.build_py import build_py as _build_py
from setuptools.command.sdist import sdist as _sdist


PACKAGE_ROOT = Path(__file__).resolve().parent
NATIVE_ROOT = PACKAGE_ROOT / "native"
PUBLIC_ROOT = PACKAGE_ROOT.parent.parent
PROTOCOL_ROOT = PUBLIC_ROOT / "protocol"
if not PROTOCOL_ROOT.is_dir():
    PROTOCOL_ROOT = PACKAGE_ROOT / "protocol"
CORE_ROOT = PUBLIC_ROOT / "clients" / "core"
if not CORE_ROOT.is_dir():
    CORE_ROOT = PACKAGE_ROOT / "core"
VALUE_ROOT = PUBLIC_ROOT / "clients" / "value"
if not VALUE_ROOT.is_dir():
    VALUE_ROOT = PACKAGE_ROOT / "value"
CLIENTS_ROOT = PUBLIC_ROOT / "clients"
CLIENT_GENERATOR = CLIENTS_ROOT / "generate.ts"
CLIENT_GENERATOR_ROOT = CLIENTS_ROOT / "generator"
CLIENT_MODEL_ROOT = CLIENTS_ROOT / "model"
GENERATED_MODULES = (
    "smithy_api.py",
    "smithy_operations.py",
    "smithy_contract.py",
    "smithy_native_abi.py",
)
if not CLIENT_GENERATOR.is_file():
    CLIENTS_ROOT = PACKAGE_ROOT / "clients"
    CLIENT_GENERATOR = CLIENTS_ROOT / "generate.ts"
    CLIENT_GENERATOR_ROOT = CLIENTS_ROOT / "generator"
    CLIENT_MODEL_ROOT = CLIENTS_ROOT / "model"


def generated_module_paths() -> tuple[Path, ...]:
    """Return every generated Python module required by the package facade."""

    generated_root = PACKAGE_ROOT / "src" / "openkache" / "_generated"
    return tuple(generated_root / module for module in GENERATED_MODULES)


def native_library_name() -> str:
    if os.name == "nt":
        return "openkache_client_python_native.dll"
    if sys.platform == "darwin":
        return "libopenkache_client_python_native.dylib"
    return "libopenkache_client_python_native.so"


def generate_smithy_contract(*, force: bool = False) -> None:
    """Generate Python contracts from the scoped wire and client Smithy models.

    Source distributions contain the generated modules so installing from an
    sdist does not require the repository's Bun/Smithy toolchain just to
    regenerate an identical Python API. A maintainer can still force
    regeneration with ``python setup.py generate_smithy`` or
    ``OPENKACHE_REGENERATE_SMITHY=1``.
    """

    if not force and all(path.is_file() for path in generated_module_paths()):
        return

    if not CLIENT_GENERATOR.is_file() or not CLIENT_GENERATOR_ROOT.is_dir():
        raise RuntimeError(
            "Python package builds require the bundled clients/generate.ts, "
            "clients/generator, clients/model, and protocol/model Smithy sources."
        )
    environment = os.environ.copy()
    environment["OPENKACHE_GENERATION_TARGET"] = "python"
    generated_root = PACKAGE_ROOT / "src" / "openkache" / "_generated"
    environment["OPENKACHE_PYTHON_API_OUTPUT"] = str(generated_root / "smithy_api.py")
    environment["OPENKACHE_PYTHON_CONTRACT_OUTPUT"] = str(
        generated_root / "smithy_contract.py"
    )
    environment["OPENKACHE_PYTHON_OPERATIONS_OUTPUT"] = str(
        generated_root / "smithy_operations.py"
    )
    environment["OPENKACHE_PYTHON_NATIVE_ABI_OUTPUT"] = str(
        generated_root / "smithy_native_abi.py"
    )
    try:
        subprocess.run(
            [
                os.environ.get("BUN", "bun"),
                str(CLIENT_GENERATOR),
            ],
            cwd=CLIENTS_ROOT,
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
        generate_smithy_contract(force=True)


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
        if destination.is_file():
            destination.unlink()
        legacy_destination = destination.with_name("_native.so")
        if legacy_destination.is_file():
            legacy_destination.unlink()
        configured_source = os.environ.get("OPENKACHE_CLIENT_NATIVE")
        if configured_source:
            source = Path(configured_source)
        else:
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
            raise RuntimeError(f"native build did not produce {source}")
        shutil.copyfile(source, destination)


class build_py(_build_py):
    """Run the native build after Python sources have been staged."""

    def run(self) -> None:
        generate_smithy_contract(
            force=os.environ.get("OPENKACHE_REGENERATE_SMITHY") == "1"
        )
        super().run()
        copy_license(Path(self.build_lib) / "openkache" / "LICENSE")
        self.run_command("build_native")


class sdist(_sdist):
    """Bundle shared Rust sources so source builds compile native code per platform."""

    def run(self) -> None:
        self.run_command("generate_smithy")
        super().run()

    def make_distribution(self) -> None:
        """Build the release tree outside the checkout.

        ``setuptools`` normally creates ``<name>-<version>`` beside
        ``setup.py`` while assembling an sdist.  The public checkout is also
        a Bazel source tree, so that transient directory can race Bazel's
        package scanner during parallel CI.  Keep the archive assembly in a
        temporary directory and publish only the finished archive.
        """

        staging_root = Path(
            tempfile.mkdtemp(prefix=f"{self.distribution.get_fullname()}-")
        )
        try:
            with self._remove_os_link():
                base_dir = self.distribution.get_fullname()
                release_root = staging_root / base_dir
                base_name = os.path.join(self.dist_dir, base_dir)
                self.make_release_tree(str(release_root), self.filelist.files)
                archive_files = []
                if "tar" in self.formats:
                    self.formats.append(self.formats.pop(self.formats.index("tar")))
                for fmt in self.formats:
                    archive_file = self.make_archive(
                        base_name,
                        fmt,
                        root_dir=str(staging_root),
                        base_dir=base_dir,
                        owner=self.owner,
                        group=self.group,
                    )
                    archive_files.append(archive_file)
                    self.distribution.dist_files.append(("sdist", "", archive_file))
                self.archive_files = archive_files
        finally:
            if not self.keep_temp:
                shutil.rmtree(staging_root, ignore_errors=True)

    def make_release_tree(self, base_dir: str, files: list[str]) -> None:
        super().make_release_tree(base_dir, files)
        release_root = Path(base_dir)
        copy_license(release_root / "LICENSE")
        source_ignore = shutil.ignore_patterns(
            "__pycache__",
            "*.pyc",
            "generated_local",
            "target",
        )
        shutil.copytree(CORE_ROOT, release_root / "core", ignore=source_ignore)
        shutil.copytree(VALUE_ROOT, release_root / "value", ignore=source_ignore)
        shutil.copytree(PROTOCOL_ROOT, release_root / "protocol", ignore=source_ignore)
        (release_root / "clients").mkdir(parents=True, exist_ok=True)
        for generator_source in CLIENTS_ROOT.glob("*.ts"):
            shutil.copy2(generator_source, release_root / "clients" / generator_source.name)
        shutil.copytree(
            CLIENT_GENERATOR_ROOT,
            release_root / "clients" / "generator",
            ignore=source_ignore,
        )
        shutil.copytree(
            CLIENT_MODEL_ROOT,
            release_root / "clients" / "model",
            ignore=source_ignore,
        )
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
        _replace_workspace_edition(release_root / "core" / "Cargo.toml")
        _replace_workspace_edition(release_root / "value" / "Cargo.toml")
        _replace_workspace_edition(release_root / "protocol" / "Cargo.toml")


def _replace_path_dependency(path: Path, source: str, destination: str) -> None:
    content = path.read_text(encoding="utf-8")
    if content.count(source) != 1:
        raise RuntimeError(
            f"expected exactly one {source!r} dependency path in {path}"
        )
    path.write_text(content.replace(source, destination), encoding="utf-8")


def _replace_workspace_edition(path: Path) -> None:
    """Make a flattened sdist crate independent of the checkout workspace."""

    source = "edition.workspace = true"
    destination = 'edition = "2024"'
    content = path.read_text(encoding="utf-8")
    if content.count(source) != 1:
        raise RuntimeError(f"expected exactly one {source!r} in {path}")
    path.write_text(content.replace(source, destination), encoding="utf-8")


def copy_license(destination: Path) -> None:
    """Copy the public Apache-2.0 license into a build or sdist tree."""

    source = PACKAGE_ROOT / "LICENSE"
    if not source.is_file():
        source = PUBLIC_ROOT / "clients" / "LICENSE"
    if not source.is_file():
        raise RuntimeError(
            "OpenKache package builds require the Apache-2.0 license at "
            f"{source}"
        )
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)


class bdist_wheel(_bdist_wheel):
    """Build a PyPI-compatible platform wheel without a Python ABI lock."""

    def finalize_options(self) -> None:
        super().finalize_options()
        self.root_is_pure = False

    def get_tag(self) -> tuple[str, str, str]:
        _, _, platform = super().get_tag()
        if platform.startswith("linux_"):
            # PyPI rejects generic ``linux_*`` wheel tags. The release runner
            # is Ubuntu 24.04 and the native adapter's highest glibc symbol is
            # GLIBC_2.38, so advertise the matching PEP 600 policy tag.
            platform = f"manylinux_2_38_{platform.removeprefix('linux_')}"
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
