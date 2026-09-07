#!/usr/bin/env python3
"""Check exported animation timing with libavif's independent decoder."""
import argparse
import hashlib
from pathlib import Path
import re
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('artifacts', type=Path)
parser.add_argument('--decoder', default='avifdec')
parser.add_argument('--manifest', required=True, type=Path)
args = parser.parse_args()
files = sorted(args.artifacts.glob('*.avif'))
assert len(files) == 16, f'Expected 16 timing files, got {len(files)}'
rows = ['file\tsha256\ttimescale\tdurations']
for path in files:
    _, clock, kind = path.stem.split('-')
    clock, kind = int(clock), int(kind)
    durations = {30000: [1001, 2002], 1000000: [1, 7], 4294967295: [4294967295, 4294967295], 1000: [20, 30]}[clock]
    result = subprocess.run([args.decoder, '-j', '1', '--info', str(path)], capture_output=True, text=True)
    output = result.stdout + result.stderr
    path.with_suffix('.libavif.txt').write_text(output)
    print(path.name, result.returncode, flush=True)
    assert result.returncode == 0, output
    assert f'{clock} timescales per second' in output, output
    assert f'({sum(durations)} timescales), 2 frames' in output, output
    timing = re.findall(r'Decoded frame \[(\d+)\] \[pts [^ ]+ \((\d+) timescales\)\] \[duration [^ ]+ \((\d+) timescales\)\]', output)
    assert timing == [('0', '0', str(durations[0])), ('1', str(durations[0]), str(durations[1]))], output
    assert '65x67' in output
    assert f'Bit Depth      : {8 if kind < 2 else 10}' in output
    rows.append('\t'.join([path.name, hashlib.sha256(path.read_bytes()).hexdigest(), str(clock), ','.join(map(str, durations))]))
args.manifest.write_text('\n'.join(rows) + '\n')
print('PASS 16 files / 32 independently decoded frames with exact durations and PTS')
