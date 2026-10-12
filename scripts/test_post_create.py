import os
from pathlib import Path
import subprocess
import tempfile
import unittest


POST_CREATE = Path(__file__).resolve().parents[1] / ".devcontainer/post-create.sh"


class PostCreateTests(unittest.TestCase):
    def run_setup(self, scenario):
        with tempfile.TemporaryDirectory(prefix="tks-post-create-") as directory:
            root = Path(directory)
            binaries = root / "bin"
            binaries.mkdir()
            rustup = binaries / "rustup"
            rustup.write_text(
                """#!/bin/bash
printf '%s\\n' "$*" >> "$TEST_ROOT/commands"
if [[ "$1 $2" == "toolchain install" ]]; then
    if [[ ! -f "$TEST_ROOT/attempted" ]]; then
        touch "$TEST_ROOT/attempted"
        case "$SCENARIO" in
            conflict|retry-failure|uninstall-failure)
                echo "error: failed to install component: 'clippy-preview-x86_64-unknown-linux-gnu', detected conflict: 'bin/cargo-clippy'" >&2
                exit 1 ;;
            network)
                echo "error: could not download file" >&2
                exit 1 ;;
        esac
    elif [[ "$SCENARIO" == retry-failure ]]; then
        echo "error: could not download file" >&2
        exit 1
    fi
elif [[ "$1 $2" == "toolchain uninstall" && "$SCENARIO" == uninstall-failure ]]; then
    echo "error: permission denied" >&2
    exit 1
fi
"""
            )
            rustup.chmod(0o755)
            for command in ("cargo", "psql"):
                stub = binaries / command
                stub.write_text(
                    '#!/bin/bash\nprintf "%s\\n" "$(basename "$0") $*" >> "$TEST_ROOT/commands"\n'
                    'echo 1\n'
                )
                stub.chmod(0o755)
            (root / "Cargo.toml").touch()
            for script in (".shared/devcontainer/post-create.sh", "scripts/blender/setup-mcp.sh"):
                path = root / script
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("#!/bin/bash\nexit 0\n")
            result = subprocess.run(
                ["bash", str(POST_CREATE)],
                cwd=root,
                env={
                    **os.environ,
                    "PATH": f"{binaries}:/usr/bin:/bin",
                    "TEST_ROOT": str(root),
                    "SCENARIO": scenario,
                },
                capture_output=True,
                text=True,
                timeout=10,
            )
            return result, (root / "commands").read_text().splitlines()

    def test_healthy_install_does_not_reinstall(self):
        result, commands = self.run_setup("healthy")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            commands[:3],
            [
                "toolchain install stable --profile minimal --component rustfmt,clippy,rust-analyzer,rust-src",
                "default stable",
                "cargo check",
            ],
        )
        self.assertNotIn("toolchain uninstall stable", commands)

    def test_conflict_reinstalls_stable_and_continues(self):
        result, commands = self.run_setup("conflict")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(commands[1], "toolchain uninstall stable")
        self.assertEqual(commands[0], commands[2])
        self.assertEqual(commands[3:5], ["default stable", "cargo check"])
        self.assertIn("detected conflict:", result.stderr)
        self.assertIn("Repairing conflicting stable", result.stderr)

    def test_network_failure_does_not_reinstall_or_continue(self):
        result, commands = self.run_setup("network")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(commands), 1)
        self.assertIn("could not download file", result.stderr)

    def test_failed_retry_stops_setup(self):
        result, commands = self.run_setup("retry-failure")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(commands), 3)
        self.assertNotIn("default stable", commands)
        self.assertIn("could not download file", result.stderr)

    def test_failed_uninstall_stops_setup(self):
        result, commands = self.run_setup("uninstall-failure")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(commands), 2)
        self.assertNotIn("default stable", commands)
        self.assertIn("permission denied", result.stderr)


if __name__ == "__main__":
    unittest.main()
