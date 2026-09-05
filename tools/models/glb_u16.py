# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marc Hoffmann (b14ckyy)

"""Rewrite a .glb so every index accessor is uint16 (Kite's 2D mesh loader reads Uint16Array).
Rebuilds the BIN chunk view by view. Usage: glb_u16.py <in.glb> <out.glb>"""
import json, struct, sys
import numpy as np

src, dst = sys.argv[1], sys.argv[2]
d = open(src, 'rb').read()
assert d[:4] == b'glTF'
jlen = struct.unpack('<I', d[12:16])[0]
js = json.loads(d[20:20 + jlen])
boff = 20 + jlen
blen, btype = struct.unpack('<II', d[boff:boff + 8]); assert btype == 0x004E4942
bin_old = d[boff + 8:boff + 8 + blen]

# which bufferViews hold indices (u32 → u16)
index_views = {}
for m in js['meshes']:
    for p in m['primitives']:
        a = js['accessors'][p['indices']]
        if a['componentType'] == 5125:
            index_views.setdefault(a['bufferView'], []).append(p['indices'])

new_bin = bytearray()
for vi, bv in enumerate(js['bufferViews']):
    assert 'byteStride' not in bv, "interleaved views not handled"
    start = bv.get('byteOffset', 0); data = bin_old[start:start + bv['byteLength']]
    if vi in index_views:
        arr = np.frombuffer(data, dtype='<u4')
        assert arr.max() < 65536, "too many vertices for uint16"
        data = arr.astype('<u2').tobytes()
        for ai in index_views[vi]:
            js['accessors'][ai]['componentType'] = 5123
            js['accessors'][ai]['byteOffset'] = js['accessors'][ai].get('byteOffset', 0) // 2
    while len(new_bin) % 4:
        new_bin += b'\0'
    bv['byteOffset'] = len(new_bin); bv['byteLength'] = len(data)
    new_bin += data
while len(new_bin) % 4:
    new_bin += b'\0'
js['buffers'][0]['byteLength'] = len(new_bin)

jbytes = json.dumps(js, separators=(',', ':')).encode()
jbytes += b' ' * (-len(jbytes) % 4)
out = bytearray(b'glTF' + struct.pack('<II', 2, 0))
out += struct.pack('<II', len(jbytes), 0x4E4F534A) + jbytes
out += struct.pack('<II', len(new_bin), 0x004E4942) + new_bin
struct.pack_into('<I', out, 8, len(out))
open(dst, 'wb').write(bytes(out))
print(f"{src.split('/')[-1]}: {len(index_views)} index views -> uint16, {len(out)} bytes")
