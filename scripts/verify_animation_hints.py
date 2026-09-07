#!/usr/bin/env python3
"""Verify exported animation hint streams with an independent decoder."""
import argparse
import hashlib
from pathlib import Path
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('artifacts', type=Path)
parser.add_argument('--decoder', default='aomdec')
parser.add_argument('--manifest', type=Path, required=True)
args = parser.parse_args()
files = sorted(args.artifacts.glob('*.obu'))
assert len(files) == 12, f'Expected 12 hint streams, got {len(files)}'
rows = ['file\tencoded_sha256\tdecoded_sha256']
outputs = {}
for path in files:
    _, depth, track, arm = path.stem.split('-')
    depth = int(depth)
    output = path.with_suffix('.decoded.yuv')
    result = subprocess.run([args.decoder, '--rawvideo', f'--output-bit-depth={depth}',
                             f'--output={output}', str(path)], capture_output=True, text=True)
    print(path.name, result.returncode, result.stderr.strip(), flush=True)
    assert result.returncode == 0, path
    decoded = output.read_bytes()
    samples = 129 * 67 + (0 if track == 'alpha' else 2 * 65 * 34)
    assert len(decoded) == 2 * samples * (1 if depth == 8 else 2), path
    outputs[depth, track, arm] = decoded
    rows.append('\t'.join([path.name, hashlib.sha256(path.read_bytes()).hexdigest(), hashlib.sha256(decoded).hexdigest()]))
for depth in (8, 10):
    for track in ('alpha', 'color'):
        assert outputs[depth, track, 'base'] == outputs[depth, track, 'neutral']
    assert outputs[depth, 'alpha', 'base'] == outputs[depth, 'alpha', 'skewed']
    assert outputs[depth, 'color', 'base'] != outputs[depth, 'color', 'skewed']
args.manifest.write_text('\n'.join(rows) + '\n')
print('PASS 12 streams / 24 frames; neutral pixels and alpha identical; hinted color differs')
