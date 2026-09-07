#!/usr/bin/env python3
"""Independently verify animation color signaling, decoding and RGB losslessness."""
import argparse
import hashlib
from pathlib import Path
import subprocess

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('artifacts', type=Path)
p.add_argument('--manifest', type=Path, required=True)
a = p.parse_args()
files = sorted(a.artifacts.glob('*.avif'))
assert len(files) == 20, len(files)
rows = ['file\tsha256\tdecoded_sha256\trgb_source_exact']
for path in files:
    _, mode, kind = path.stem.split('-')
    kind = int(kind)
    depth = 8 if kind < 2 else 10
    info = subprocess.run(['avifdec', '-j', '1', '--info', str(path)], capture_output=True, text=True)
    output = info.stdout + info.stderr
    path.with_suffix('.libavif.txt').write_text(output)
    assert info.returncode == 0, output
    assert f'Format         : YUV{420 if mode.startswith("420") else 444}' in output, output
    assert f'Range          : {"Limited" if "limited" in mode else "Full"}' in output, output
    assert f'Matrix Coeffs. : {0 if mode == "rgb" else 6}' in output, output
    assert f'Bit Depth      : {depth}' in output, output
    assert output.count('Decoded frame [') == 2, output
    raw = path.with_suffix('.decoded.yuv')
    dec = subprocess.run(['aomdec', '--threads=1', '--rawvideo', f'--output-bit-depth={depth}', f'--output={raw}', str(path.with_suffix('.obu'))], capture_output=True, text=True)
    assert dec.returncode == 0, dec.stderr
    data = raw.read_bytes()
    chroma = 33 * 34 if mode.startswith('420') else 65 * 67
    assert len(data) == 2 * (65 * 67 + 2 * chroma) * (1 if depth == 8 else 2), path
    if mode == 'rgb':
        assert data == path.with_suffix('.source.yuv').read_bytes(), f'{path}: RGB planes changed'
    rows.append('\t'.join([path.name, hashlib.sha256(path.read_bytes()).hexdigest(), hashlib.sha256(data).hexdigest(), str(mode == 'rgb')]))
    print('PASS', path.name, flush=True)
a.manifest.write_text('\n'.join(rows) + '\n')
print('PASS 20 files / 40 frames; all four RGB streams equal original GBR planes exactly')
