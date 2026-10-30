#!/usr/bin/env python3
"""Exercise the packaged launcher and its actual MCP descriptor without services."""

import io
import json
import os
from pathlib import Path
import queue
import shutil
import subprocess
import tarfile
import tempfile
import threading
import time
import unittest


ROOT = Path(__file__).resolve().parents[1]
LEGACY = json.loads((ROOT / ".claude-plugin/plugin.json").read_text())
NAME = LEGACY["name"]
SERVER = json.loads((ROOT / ".mcp.json").read_text())["mcpServers"][NAME]
BINARY = Path(SERVER["command"]).name


def executable(path, body):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(body)
    path.chmod(0o755)


class PluginTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory(prefix="plugin test ")
        self.addCleanup(self.scratch.cleanup)
        self.base = Path(self.scratch.name)
        self.plugin = self.base / "plugin root"
        self.data = self.base / "plugin data"
        self.helpers = self.base / "helpers"
        self.plugin.mkdir()
        self.helpers.mkdir()
        self.data.mkdir()
        (self.plugin / "bin").mkdir()
        shutil.copy2(ROOT / "bin" / BINARY, self.plugin / "bin" / BINARY)
        (self.plugin / "plugin.json").write_text(json.dumps({"name": NAME, "version": "1.2.3"}))
        self.env = {
            "PATH": f"{self.helpers}:/usr/bin:/bin",
            "HOME": str(self.base / "home"),
            "XDG_CONFIG_HOME": str(self.base / "config"),
            "PLUGIN_ROOT": str(self.plugin),
            "PLUGIN_DATA": str(self.data),
        }
        # A broken version lookup must never turn a test into a network request.
        executable(self.helpers / "curl", "#!/bin/bash\nexit 97\n")

    def cached(self, body="printf 'CACHED\\n'", version="1.2.3"):
        executable(self.data / "bin" / BINARY, f"#!/bin/bash\n{body}\n")
        (self.data / "bin" / f"{BINARY}.version").write_text(version + "\n")

    def launch(self):
        return subprocess.run(
            [str(self.plugin / "bin" / BINARY), "--version"],
            env=self.env, capture_output=True, text=True, timeout=10,
        )

    def assert_cached(self):
        result = self.launch()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "CACHED\n")

    def test_portable_root_and_data_override_claude_aliases(self):
        self.env["CLAUDE_PLUGIN_ROOT"] = str(self.base / "wrong root")
        self.env["CLAUDE_PLUGIN_DATA"] = str(self.base / "wrong data")
        legacy = self.plugin / ".claude-plugin"
        legacy.mkdir()
        (legacy / "plugin.json").write_text('{"version":"9.9.9"}')
        self.cached()
        self.assert_cached()
        self.assertFalse((self.base / "wrong data").exists())

    def test_system_helpers_with_sanitized_path(self):
        self.cached()
        for path in [None, "", str(self.helpers)]:
            with self.subTest(path=path):
                if path is None:
                    self.env.pop("PATH", None)
                else:
                    self.env["PATH"] = path
                self.assert_cached()

    def test_portable_data_does_not_require_home(self):
        self.env.pop("HOME")
        self.cached()
        self.assert_cached()

    def test_version_lookup_without_optional_jq(self):
        bash_env = self.base / "without-jq.sh"
        bash_env.write_text('command() { if [[ $1 == -v && $2 == jq ]]; then return 1; fi; builtin command "$@"; }\n')
        self.env["BASH_ENV"] = str(bash_env)
        self.cached()
        self.assert_cached()

    def test_user_binary_wins_without_self_recursion(self):
        executable(self.helpers / BINARY, "#!/bin/bash\nprintf 'USER\\n'\n")
        self.env["PATH"] = f"{self.plugin / 'bin'}:{self.env['PATH']}"
        result = self.launch()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "USER\n")

    def test_legacy_claude_environment_and_manifest(self):
        (self.plugin / "plugin.json").unlink()
        legacy = self.plugin / ".claude-plugin"
        legacy.mkdir()
        (legacy / "plugin.json").write_text('{"version":"1.2.3"}')
        self.env["CLAUDE_PLUGIN_ROOT"] = self.env.pop("PLUGIN_ROOT")
        self.env["CLAUDE_PLUGIN_DATA"] = self.env.pop("PLUGIN_DATA")
        self.cached()
        self.assert_cached()

    def test_cold_install_keeps_protocol_stdout_clean(self):
        archive = self.base / "release.tar.gz"
        payload = b"#!/bin/bash\nprintf 'DOWNLOADED\\n'\n"
        with tarfile.open(archive, "w:gz") as tar:
            entry = tarfile.TarInfo(BINARY)
            entry.size = len(payload)
            entry.mode = 0o755
            tar.addfile(entry, io.BytesIO(payload))
        self.env["TEST_ARCHIVE"] = str(archive)
        self.env["TMPDIR"] = str(self.base / "tmp")
        Path(self.env["TMPDIR"]).mkdir()
        executable(self.helpers / "mktemp", '#!/bin/bash\n/usr/bin/mktemp -d "$TMPDIR/download.XXXXXXXXXX"\n')
        executable(self.helpers / "uname", "#!/bin/bash\ncase $1 in -s) echo Darwin;; -m) echo arm64;; esac\n")
        executable(self.helpers / "curl", '#!/bin/bash\ncp "$TEST_ARCHIVE" "$3"\n')
        result = self.launch()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "DOWNLOADED\n")
        self.assertIn("fetching", result.stderr)
        self.assertEqual((self.data / "bin" / f"{BINARY}.version").read_text().strip(), "1.2.3")
        self.assertEqual(list(Path(self.env["TMPDIR"]).iterdir()), [])
        self.assertEqual(sorted(p.name for p in self.plugin.iterdir()), ["bin", "plugin.json"])

    @unittest.skipUnless(os.environ.get("PLUGIN_TEST_BINARY"), "set PLUGIN_TEST_BINARY to a built CLI")
    def test_committed_descriptor_initializes_real_mcp(self):
        manifest = json.loads((ROOT / "plugin.json").read_text())
        descriptor = json.loads((ROOT / "mcp.json").read_text())["mcpServers"][NAME]
        (self.plugin / "plugin.json").write_text(json.dumps(manifest))
        (self.data / "bin").mkdir()
        shutil.copy2(Path(os.environ["PLUGIN_TEST_BINARY"]).resolve(), self.data / "bin" / BINARY)
        (self.data / "bin" / f"{BINARY}.version").write_text(manifest["version"])
        if NAME == "pngx":
            config = 'url = "http://127.0.0.1:9"\ntoken = "fixture-token"\n'
            for base in [Path(self.env["XDG_CONFIG_HOME"]), Path(self.env["HOME"]) / "Library/Application Support"]:
                path = base / "pngx/config.toml"
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(config)
                path.chmod(0o600)

        def expand(value):
            return value.replace("${PLUGIN_ROOT}", str(self.plugin)).replace("${PLUGIN_DATA}", str(self.data))

        self.env.update({key: expand(value) for key, value in descriptor.get("env", {}).items()})
        command = descriptor["command"]
        command = str(self.plugin / command) if command.startswith("./") else command
        process = subprocess.Popen(
            [command, *map(expand, descriptor.get("args", []))],
            cwd=expand(descriptor.get("cwd", str(self.plugin))), env=self.env,
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        )
        messages = queue.Queue()
        stderr_lines = []

        def read_messages():
            for line in process.stdout:
                messages.put(line)
            messages.put(None)

        reader = threading.Thread(target=read_messages, daemon=True)
        errors = threading.Thread(target=lambda: stderr_lines.extend(process.stderr), daemon=True)
        reader.start()
        errors.start()
        deadline = time.monotonic() + 10

        def send(message):
            process.stdin.write(json.dumps(message) + "\n")
            process.stdin.flush()

        def response(expected_id):
            while True:
                try:
                    line = messages.get(timeout=max(0, deadline - time.monotonic()))
                except queue.Empty:
                    self.fail("MCP response deadline exceeded: " + "".join(stderr_lines))
                self.assertIsNotNone(line, "MCP closed stdout before replying: " + "".join(stderr_lines))
                message = json.loads(line)
                if message.get("id") == expected_id:
                    self.assertNotIn("error", message)
                    return message["result"]

        try:
            send({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                "protocolVersion": "2025-03-26", "capabilities": {},
                "clientInfo": {"name": "plugin-test", "version": "1"},
            }})
            self.assertIn("serverInfo", response(1))
            send({"jsonrpc": "2.0", "method": "notifications/initialized"})
            send({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
            self.assertTrue(response(2)["tools"])
        finally:
            process.terminate()
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
            reader.join(timeout=1)
            errors.join(timeout=1)
            process.stdin.close()
            process.stdout.close()
            process.stderr.close()


if __name__ == "__main__":
    unittest.main()
