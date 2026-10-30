# /// script
# requires-python = ">=3.11"
# dependencies = ["jsonschema==4.25.1", "PyYAML==6.0.3"]
# ///
"""Validate portable packaging, Agent Skills metadata, and release versions."""

import json
import os
from pathlib import Path
import re
import tomllib

import jsonschema
import yaml


ROOT = Path(__file__).resolve().parents[1]


def read_json(path):
    return json.loads(path.read_text())


def validate_skill(path):
    text = path.read_text()
    assert text.startswith("---\n"), f"{path}: missing frontmatter"
    fields = yaml.safe_load(text.split("---", 2)[1])
    allowed = {"name", "description", "license", "compatibility", "metadata", "allowed-tools"}
    assert isinstance(fields, dict) and not fields.keys() - allowed, f"{path}: unsupported frontmatter"
    name = fields.get("name", "")
    assert isinstance(name, str) and re.fullmatch(r"[a-z0-9]+(?:-[a-z0-9]+)*", name), f"{path}: invalid skill name"
    assert len(name) <= 64 and name == path.parent.name, f"{path}: name must match directory"
    description = fields.get("description", "")
    assert isinstance(description, str) and 1 <= len(description.strip()) <= 1024, f"{path}: invalid description"
    for key, limit in [("compatibility", 500), ("license", None), ("allowed-tools", None)]:
        if key in fields:
            assert isinstance(fields[key], str) and fields[key].strip(), f"{path}: invalid {key}"
            assert limit is None or len(fields[key]) <= limit, f"{path}: {key} too long"
    if "metadata" in fields:
        metadata = fields["metadata"]
        assert isinstance(metadata, dict) and all(isinstance(k, str) and isinstance(v, str) for k, v in metadata.items()), f"{path}: metadata must map strings to strings"


def main():
    plugin = read_json(ROOT / "plugin.json")
    mcp = read_json(ROOT / "mcp.json")
    for name, value in [("plugin", plugin), ("mcp", mcp)]:
        schema = read_json(ROOT / "scripts/schemas" / f"{name}-1.0.0.json")
        jsonschema.Draft202012Validator(schema).validate(value)
    for server in mcp["mcpServers"].values():
        assert server["type"] == "stdio", "these plugins package stdio servers"
        command = server["command"]
        assert command.startswith("./") and "${" not in command, "bundled command must be plugin-relative"
        executable = (ROOT / command).resolve()
        assert executable.is_relative_to(ROOT) and executable.is_file() and os.access(executable, os.X_OK), "launcher must be executable inside package"
        for value in [*server.get("args", []), *server.get("env", {}).values(), server.get("cwd", "")]:
            assert set(re.findall(r"\$\{[^}]+\}", value)) <= {"${PLUGIN_ROOT}", "${PLUGIN_DATA}"}, "unresolved nonportable placeholder"
    hooks = plugin["extensions"]["com.openai"]["hooks"]
    assert hooks.startswith("./com.openai/") and (ROOT / hooks).is_file(), "missing namespaced OpenAI hooks"
    skills = list((ROOT / "skills").glob("*/SKILL.md"))
    assert skills, "package must contain its skill"
    for path in skills:
        validate_skill(path)
    crate = ROOT / "Cargo.toml"
    cargo = tomllib.loads(crate.read_text())
    if "package" not in cargo:
        cargo = tomllib.loads((ROOT / "crates" / plugin["name"] / "Cargo.toml").read_text())
    version = cargo["package"]["version"]
    assert plugin["version"] == version, "portable version differs from crate"
    assert read_json(ROOT / ".claude-plugin/plugin.json")["version"] == version, "Claude version differs from crate"
    assert read_json(ROOT / ".claude-plugin/marketplace.json")["plugins"][0]["version"] == version, "marketplace version differs from crate"
    assert re.search(r"^## \[" + re.escape(version) + r"\] - \d{4}-\d{2}-\d{2}$", (ROOT / "CHANGELOG.md").read_text(), re.MULTILINE), "missing versioned release notes"
    print(f"Validated {plugin['name']} {version}: portable manifests, skills, and versions")


if __name__ == "__main__":
    main()
