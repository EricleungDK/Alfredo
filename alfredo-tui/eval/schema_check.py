"""Validate a document against the JSON Schema subset used by schema.json (stdlib only).

Supported keywords: type (name or list), enum, const, required, properties,
additionalProperties (bool or schema), items, minimum, maximum, minLength, and a local
`$ref` to `#/$defs/NAME`. Unknown keywords are ignored, so keep schema.json to this subset.
"""
import json
from pathlib import Path

SCHEMA_PATH = Path(__file__).resolve().parent / 'schema.json'


def _is(value, name):
    if name == 'null':
        return value is None
    if name == 'boolean':
        return isinstance(value, bool)
    if name == 'integer':
        return isinstance(value, int) and not isinstance(value, bool)
    if name == 'number':
        return isinstance(value, (int, float)) and not isinstance(value, bool)
    if name == 'string':
        return isinstance(value, str)
    if name == 'array':
        return isinstance(value, list)
    if name == 'object':
        return isinstance(value, dict)
    raise ValueError(f'unsupported type {name!r}')


def validate(document, schema=None):
    """Return a list of human-readable violations (empty means valid)."""
    root = schema if schema is not None else json.loads(SCHEMA_PATH.read_text(encoding='utf-8'))
    errors = []

    def walk(value, node, path):
        if '$ref' in node:
            target = root
            for part in node['$ref'].removeprefix('#/').split('/'):
                target = target[part]
            node = target
        if 'type' in node:
            names = node['type'] if isinstance(node['type'], list) else [node['type']]
            if not any(_is(value, name) for name in names):
                errors.append(f'{path}: expected {"|".join(names)}, got {type(value).__name__}')
                return
        if 'const' in node and value != node['const']:
            errors.append(f'{path}: expected {node["const"]!r}, got {value!r}')
        if 'enum' in node and value not in node['enum']:
            errors.append(f'{path}: {value!r} not in {node["enum"]}')
        if isinstance(value, (int, float)) and not isinstance(value, bool):
            if 'minimum' in node and value < node['minimum']:
                errors.append(f'{path}: {value} < minimum {node["minimum"]}')
            if 'maximum' in node and value > node['maximum']:
                errors.append(f'{path}: {value} > maximum {node["maximum"]}')
        if isinstance(value, str) and len(value) < node.get('minLength', 0):
            errors.append(f'{path}: shorter than {node["minLength"]}')
        if isinstance(value, list) and 'items' in node:
            for index, item in enumerate(value):
                walk(item, node['items'], f'{path}[{index}]')
        if isinstance(value, dict):
            for key in node.get('required', []):
                if key not in value:
                    errors.append(f'{path}: missing required {key!r}')
            properties = node.get('properties', {})
            extra = node.get('additionalProperties', True)
            for key, item in value.items():
                if key in properties:
                    walk(item, properties[key], f'{path}.{key}')
                elif extra is False:
                    errors.append(f'{path}: unexpected property {key!r}')
                elif isinstance(extra, dict):
                    walk(item, extra, f'{path}.{key}')

    walk(document, root, '$')
    return errors
