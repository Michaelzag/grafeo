"""Parse .gtest YAML files into structured test cases.

Mirrors the Rust build.rs parser in crates/grafeo-spec-tests/build.rs so that
the Python runner exercises the exact same test definitions.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Dict, List, Optional

# ---------------------------------------------------------------------------
# Try PyYAML first, fall back to our own line-based parser
# ---------------------------------------------------------------------------
try:
    import yaml as _yaml

    HAS_YAML = True
except ImportError:
    HAS_YAML = False


# ---------------------------------------------------------------------------
# Data structures
# ---------------------------------------------------------------------------


@dataclass
class Meta:
    language: str = "gql"
    model: str = ""
    section: str = ""
    title: str = ""
    dataset: str = "empty"
    requires: List[str] = field(default_factory=list)
    tags: List[str] = field(default_factory=list)


@dataclass
class Expect:
    rows: List[List[str]] = field(default_factory=list)
    ordered: bool = False
    count: Optional[int] = None
    empty: bool = False
    error: Optional[str] = None
    hash: Optional[str] = None
    precision: Optional[int] = None
    columns: List[str] = field(default_factory=list)


@dataclass
class TestCase:
    name: str = ""
    query: Optional[str] = None
    statements: List[str] = field(default_factory=list)
    setup: List[str] = field(default_factory=list)
    params: Dict[str, object] = field(default_factory=dict)
    tags: List[str] = field(default_factory=list)
    requires: List[str] = field(default_factory=list)
    skip: Optional[str] = None
    expect: Expect = field(default_factory=Expect)
    variants: Dict[str, str] = field(default_factory=dict)
    language: Optional[str] = None


@dataclass
class GtestFile:
    meta: Meta
    tests: List[TestCase]


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------


def parse_gtest_file(path: Path) -> GtestFile:
    """Parse a .gtest file and return a GtestFile with meta + test cases.

    Tries PyYAML first but falls back to the line-based parser when the
    file contains constructs that are valid in .gtest but not in strict YAML
    (e.g. unquoted strings with embedded colons).
    """
    content = path.read_text(encoding="utf-8")
    if HAS_YAML:
        try:
            return _parse_with_yaml(content, path)
        except Exception:
            # .gtest files may contain bare strings with colons that YAML
            # rejects.  Fall through to the line-based parser.
            pass
    return _parse_line_based(content, path)


# ---------------------------------------------------------------------------
# YAML-based parser (preferred)
# ---------------------------------------------------------------------------


def _parse_with_yaml(content: str, path: Path) -> GtestFile:
    data = _yaml.safe_load(content)
    if not isinstance(data, dict):
        raise ValueError(f"Expected a YAML mapping at top level in {path}")

    meta = _parse_meta_dict(data.get("meta", {}))
    raw_tests = data.get("tests", [])
    tests: List[TestCase] = []
    for raw in raw_tests:
        tests.append(_parse_test_dict(raw))
    return GtestFile(meta=meta, tests=tests)


def _parse_meta_dict(d: dict) -> Meta:
    if d is None:
        return Meta()
    m = Meta()
    m.language = str(d.get("language", "gql"))
    m.model = str(d.get("model", ""))
    m.section = str(d.get("section", ""))
    m.title = str(d.get("title", ""))
    m.dataset = str(d.get("dataset", "empty"))
    m.requires = _as_string_list(d.get("requires", []))
    m.tags = _as_string_list(d.get("tags", []))
    return m


def _parse_test_dict(d: dict) -> TestCase:
    tc = TestCase()
    tc.name = str(d.get("name", ""))
    tc.skip = d.get("skip")
    if tc.skip is not None:
        tc.skip = str(tc.skip)
    tc.tags = _as_string_list(d.get("tags", []))
    tc.requires = _as_string_list(d.get("requires", []))

    # query: may be a string or a multi-line block
    q = d.get("query")
    if q is not None:
        tc.query = str(q).strip()

    # setup / statements: lists of strings
    tc.setup = _as_string_list(d.get("setup", []))
    tc.statements = _as_string_list(d.get("statements", []))

    # params: keep original types from YAML (bool, int, float) so
    # _coerce_params can pass them through without lossy str() conversion
    raw_params = d.get("params", {})
    if isinstance(raw_params, dict):
        tc.params = {str(k): v for k, v in raw_params.items()}

    # per-test language override (e.g. "graphql-rdf")
    lang = d.get("language")
    if lang is not None:
        tc.language = str(lang)

    # variants
    raw_variants = d.get("variants", {})
    if isinstance(raw_variants, dict):
        tc.variants = {str(k): str(v).strip() for k, v in raw_variants.items()}

    # expect
    raw_expect = d.get("expect", {})
    if isinstance(raw_expect, dict):
        tc.expect = _parse_expect_dict(raw_expect)

    return tc


def _parse_expect_dict(d: dict) -> Expect:
    e = Expect()
    e.ordered = bool(d.get("ordered", False))
    e.empty = bool(d.get("empty", False))

    count = d.get("count")
    if count is not None:
        e.count = int(count)

    error = d.get("error")
    if error is not None:
        e.error = str(error)

    hash_val = d.get("hash")
    if hash_val is not None:
        e.hash = str(hash_val)

    precision = d.get("precision")
    if precision is not None:
        e.precision = int(precision)

    e.columns = _as_string_list(d.get("columns", []))

    # rows: list of lists, each element becomes a string for comparison
    raw_rows = d.get("rows", [])
    for raw_row in raw_rows:
        if isinstance(raw_row, list):
            e.rows.append([_value_to_string(v) for v in raw_row])
        else:
            # Single-column shorthand
            e.rows.append([_value_to_string(raw_row)])

    return e


# ---------------------------------------------------------------------------
# Line-based fallback parser
# ---------------------------------------------------------------------------


def _parse_line_based(content: str, path: Path) -> GtestFile:
    """Minimal line-based parser for when PyYAML is not installed.

    This handles the subset of YAML used by .gtest files:
    top-level ``meta:`` / ``tests:`` blocks, inline ``[a, b]`` lists,
    block scalars (``|``), and ``- name:`` list items.
    """
    lines = content.splitlines()
    idx = _skip_blank_and_comments(lines, 0)

    # Parse meta block
    meta, idx = _lb_parse_meta(lines, idx)
    idx = _skip_blank_and_comments(lines, idx)

    # Parse tests block
    tests, idx = _lb_parse_tests(lines, idx)

    return GtestFile(meta=meta, tests=tests)


def _lb_parse_meta(lines: List[str], idx: int) -> tuple:
    meta = Meta()
    if idx < len(lines) and lines[idx].strip() == "meta:":
        idx += 1
    while idx < len(lines):
        line = lines[idx]
        trimmed = line.strip()
        if not trimmed or trimmed.startswith("#"):
            idx += 1
            continue
        if not line[0].isspace():
            break
        key, value = _lb_parse_kv(trimmed)
        if key == "language":
            meta.language = value
        elif key == "model":
            meta.model = value
        elif key == "section":
            meta.section = _unquote(value)
        elif key == "title":
            meta.title = value
        elif key == "dataset":
            meta.dataset = value
        elif key == "requires":
            meta.requires = _lb_parse_yaml_list(value)
        elif key == "tags":
            meta.tags = _lb_parse_yaml_list(value)
        idx += 1
    return meta, idx


def _lb_parse_tests(lines: List[str], idx: int) -> tuple:
    tests: List[TestCase] = []
    if idx < len(lines) and lines[idx].strip() == "tests:":
        idx += 1
    while idx < len(lines):
        idx = _skip_blank_and_comments(lines, idx)
        if idx >= len(lines):
            break
        trimmed = lines[idx].strip()
        if trimmed.startswith("- name:"):
            tc, idx = _lb_parse_single_test(lines, idx)
            tests.append(tc)
        else:
            break
    return tests, idx


def _lb_parse_single_test(lines: List[str], idx: int) -> tuple:
    tc = TestCase()
    first = lines[idx].strip()
    # "- name: foo"
    _, name_val = _lb_parse_kv(first[2:])  # strip "- "
    tc.name = _unquote(name_val)
    idx += 1

    while idx < len(lines):
        trimmed = lines[idx].strip()
        if trimmed.startswith("#"):
            idx += 1
            continue
        if trimmed.startswith("- name:"):
            break
        if not trimmed:
            idx += 1
            continue

        key, value = _lb_parse_kv(trimmed)
        if key == "query":
            if value == "|":
                block, idx = _lb_parse_block_scalar(lines, idx)
                tc.query = block
            else:
                tc.query = _unquote(value)
                idx += 1
        elif key == "skip":
            tc.skip = _unquote(value)
            idx += 1
        elif key == "setup":
            idx += 1
            tc.setup, idx = _lb_parse_string_list(lines, idx)
        elif key == "statements":
            idx += 1
            tc.statements, idx = _lb_parse_string_list(lines, idx)
        elif key == "tags":
            tc.tags = _lb_parse_yaml_list(value)
            idx += 1
        elif key == "requires":
            tc.requires = _lb_parse_yaml_list(value)
            idx += 1
        elif key == "params":
            idx += 1
            tc.params, idx = _lb_parse_params(lines, idx)
        elif key == "expect":
            idx += 1
            tc.expect, idx = _lb_parse_expect(lines, idx)
        elif key == "variants":
            idx += 1
            tc.variants, idx = _lb_parse_variants(lines, idx)
        else:
            idx += 1

    return tc, idx


def _lb_parse_expect(lines: List[str], idx: int) -> tuple:
    e = Expect()
    while idx < len(lines):
        trimmed = lines[idx].strip()
        if not trimmed or trimmed.startswith("#"):
            idx += 1
            continue
        if trimmed.startswith("- name:"):
            break
        if not lines[idx][0].isspace():
            break

        key, value = _lb_parse_kv(trimmed)
        if key == "ordered":
            e.ordered = value == "true"
            idx += 1
        elif key == "count":
            e.count = int(value)
            idx += 1
        elif key == "empty":
            e.empty = value == "true"
            idx += 1
        elif key == "error":
            e.error = _unquote(value)
            idx += 1
        elif key == "hash":
            e.hash = _unquote(value)
            idx += 1
        elif key == "precision":
            e.precision = int(value)
            idx += 1
        elif key == "columns":
            e.columns = _lb_parse_yaml_list(value)
            idx += 1
        elif key == "rows":
            idx += 1
            e.rows, idx = _lb_parse_rows(lines, idx)
        else:
            break
    return e, idx


def _lb_parse_rows(lines: List[str], idx: int) -> tuple:
    rows: List[List[str]] = []
    while idx < len(lines):
        trimmed = lines[idx].strip()
        if not trimmed or trimmed.startswith("#"):
            idx += 1
            continue
        if trimmed.startswith("- ["):
            inner = trimmed[2:]  # strip "- "
            values = _lb_parse_inline_list(inner)
            rows.append(values)
            idx += 1
        else:
            break
    return rows, idx


def _lb_parse_string_list(lines: List[str], idx: int) -> tuple:
    items: List[str] = []
    while idx < len(lines):
        trimmed = lines[idx].strip()
        if not trimmed or trimmed.startswith("#"):
            idx += 1
            continue
        if trimmed.startswith("- "):
            value = trimmed[2:]
            if value == "|":
                block, idx = _lb_parse_block_scalar(lines, idx)
                items.append(block)
            else:
                items.append(_unquote(value))
                idx += 1
        else:
            break
    return items, idx


def _lb_parse_params(lines: List[str], idx: int) -> tuple:
    params: Dict[str, str] = {}
    while idx < len(lines):
        trimmed = lines[idx].strip()
        if not trimmed or trimmed.startswith("#"):
            idx += 1
            continue
        indent = len(lines[idx]) - len(lines[idx].lstrip())
        if indent < 6 and not trimmed.startswith("- name:"):
            key, value = _lb_parse_kv(trimmed)
            params[key] = _unquote(value)
            idx += 1
        else:
            break
    return params, idx


def _lb_parse_variants(lines: List[str], idx: int) -> tuple:
    variants: Dict[str, str] = {}
    while idx < len(lines):
        trimmed = lines[idx].strip()
        if not trimmed or trimmed.startswith("#"):
            idx += 1
            continue
        indent = len(lines[idx]) - len(lines[idx].lstrip())
        if indent >= 6:
            key, value = _lb_parse_kv(trimmed)
            if value == "|":
                block, idx = _lb_parse_block_scalar(lines, idx)
                variants[key] = block
            else:
                variants[key] = _unquote(value)
                idx += 1
        else:
            break
    return variants, idx


def _lb_parse_block_scalar(lines: List[str], idx: int) -> tuple:
    """Parse a YAML block scalar (line ending with ``|``)."""
    idx += 1  # skip the ``|`` line
    if idx >= len(lines):
        return "", idx
    block_indent = len(lines[idx]) - len(lines[idx].lstrip())
    parts: List[str] = []
    while idx < len(lines):
        line = lines[idx]
        trimmed = line.strip()
        if not trimmed:
            parts.append("")
            idx += 1
            continue
        current_indent = len(line) - len(line.lstrip())
        if current_indent < block_indent:
            break
        parts.append(line[block_indent:])
        idx += 1
    text = "\n".join(parts).rstrip()
    return text, idx


# ---------------------------------------------------------------------------
# Shared helpers
# ---------------------------------------------------------------------------


def _skip_blank_and_comments(lines: List[str], idx: int) -> int:
    while idx < len(lines):
        trimmed = lines[idx].strip()
        if not trimmed or trimmed.startswith("#"):
            idx += 1
        else:
            break
    return idx


def _lb_parse_kv(s: str) -> tuple:
    """Split ``key: value`` respecting quotes."""
    in_single = False
    in_double = False
    for i, c in enumerate(s):
        if c == "'" and not in_double:
            in_single = not in_single
        elif c == '"' and not in_single:
            in_double = not in_double
        elif c == ":" and not in_single and not in_double:
            key = s[:i].strip()
            value = s[i + 1 :].strip()
            if key:
                return key, value
    return s.strip(), ""


def _unquote(s: str) -> str:
    """Strip surrounding quotes and unescape YAML-level escapes only.

    Do NOT process ``\\n`` or ``\\t`` here: those are GQL string escapes
    that the engine's parser handles via ``unescape_string()``.
    """
    s = s.strip()
    if len(s) >= 2 and (
        (s[0] == '"' and s[-1] == '"') or (s[0] == "'" and s[-1] == "'")
    ):
        inner = s[1:-1]
        # Use a sentinel to avoid order-dependent replacement issues
        return (
            inner.replace("\\\\", "\x00")
            .replace('\\"', '"')
            .replace("\\'", "'")
            .replace("\x00", "\\")
        )
    return s


def _lb_parse_yaml_list(s: str) -> List[str]:
    s = s.strip()
    if s == "[]" or not s:
        return []
    if s.startswith("[") and s.endswith("]"):
        inner = s[1:-1]
        return [_unquote(v.strip()) for v in inner.split(",") if v.strip()]
    return [_unquote(s)]


def _lb_parse_inline_list(s: str) -> List[str]:
    """Parse ``[a, b, c]`` respecting nested brackets and quotes."""
    s = s.strip()
    if not s.startswith("[") or not s.endswith("]"):
        return [_unquote(s)]
    inner = s[1:-1]

    items: List[str] = []
    current: List[str] = []
    depth = 0
    in_single = False
    in_double = False

    for c in inner:
        if c == "'" and not in_double and depth == 0:
            in_single = not in_single
            current.append(c)
        elif c == '"' and not in_single and depth == 0:
            in_double = not in_double
            current.append(c)
        elif c in "[{" and not in_single and not in_double:
            depth += 1
            current.append(c)
        elif c in "]}" and not in_single and not in_double:
            depth -= 1
            current.append(c)
        elif c == "," and depth == 0 and not in_single and not in_double:
            items.append(_unquote("".join(current).strip()))
            current = []
        else:
            current.append(c)

    last = "".join(current).strip()
    if last:
        items.append(_unquote(last))

    return items


def _as_string_list(val) -> List[str]:
    """Coerce a YAML value to a list of strings."""
    if val is None:
        return []
    if isinstance(val, str):
        return [val]
    if isinstance(val, list):
        return [str(v).strip() if v is not None else "" for v in val]
    return [str(val)]


def _value_to_string(val) -> str:
    """Convert a YAML-parsed Python value to the canonical string the Rust
    runner would produce.  This is the Python equivalent of
    ``value_to_string`` in ``grafeo-spec-tests/src/lib.rs``.
    """
    if val is None:
        return "null"
    if isinstance(val, bool):
        return "true" if val else "false"
    if isinstance(val, int):
        return str(val)
    if isinstance(val, float):
        if val != val:  # NaN
            return "NaN"
        if val == float("inf"):
            return "Infinity"
        if val == float("-inf"):
            return "-Infinity"
        # Rust's Display for f64 drops ".0" for whole numbers.
        if val == int(val) and abs(val) < 2**53:
            return str(int(val))
        return str(val)
    if isinstance(val, list):
        inner = ", ".join(_value_to_string(v) for v in val)
        return f"[{inner}]"
    if isinstance(val, dict):
        entries = sorted(f"{k}: {_value_to_string(v)}" for k, v in val.items())
        return "{" + ", ".join(entries) + "}"
    return str(val)
