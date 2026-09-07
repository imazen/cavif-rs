#!/usr/bin/env python3
"""Decode the exported animation coding fixture and enforce exact lossless planes."""
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
assert len(files) == 16, f'Expected 16 streams from animation_coding_options_reach_color_and_lossless_alpha, got {len(files)}'
rows = ['file\tencoded_sha256\tdecoded_sha256\tlossless_source_equal']
for path in files:
    depth = int(path.stem.split('-')[1])
    output = path.with_suffix('.decoded.yuv')
    result = subprocess.run([args.decoder, '--rawvideo', f'--output-bit-depth={depth}',
                             f'--output={output}', str(path)], capture_output=True, text=True)
    print(path.name, 'rc=', result.returncode, result.stderr.strip(), flush=True)
    assert result.returncode == 0, path
    source = args.artifacts / ('-'.join(path.stem.split('-')[:3]) + '.source.yuv')
    # Every stream contains two frames; the generated source contains their
    # visible planes in the same order, at the same native sample depth.
    decoded = output.read_bytes()
    expected = source.read_bytes()
    assert len(decoded) == len(expected), path
    lossless = 'lossless' in path.stem
    if lossless:
        assert decoded == expected, f'{path.name}: lossless samples differ from source'
    rows.append('\t'.join([path.name, hashlib.sha256(path.read_bytes()).hexdigest(),
                           hashlib.sha256(decoded).hexdigest(), 'true' if lossless else 'not_required']))
args.manifest.write_text('\n'.join(rows) + '\n')
print('PASS 16 streams / 32 frames; 8 lossless streams exactly equal their source planes')
