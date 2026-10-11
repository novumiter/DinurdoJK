"""Extract skeleton definitions and one reference frame, without gameplay clips.

Input: the original GLA files supplied in Development Docs/Models.
Output: small reference GLAs embedded by jka-assets. Run from the repo root.
"""
import hashlib
import struct
from pathlib import Path

root = Path(__file__).resolve().parent.parent
out = root / "crates/jka-assets/src/skeleton"
out.mkdir(exist_ok=True)
for rig in ("jka", "jk2"):
    source = root / f"Development Docs/Models/{rig} _humanoid.gla"
    data = source.read_bytes()
    frames, frame_offset, bones, pool, skeleton, _ = struct.unpack_from("<6i", data, 76)
    assert data[:4] == b"2LGA" and bones == {"jka": 53, "jk2": 72}[rig]
    end = max(100 + struct.unpack_from("<i", data, 100 + 4*i)[0] + 172
              + 4*struct.unpack_from("<i", data, 100 + struct.unpack_from("<i", data, 100+4*i)[0] + 168)[0]
              for i in range(bones))
    result = bytearray(data[:end])
    new_frames = len(result)
    result.extend(b"".join(i.to_bytes(3, "little") for i in range(bones)))
    new_pool = len(result)
    for i in range(bones):
        index = int.from_bytes(data[frame_offset+3*i:frame_offset+3*i+3], "little")
        result.extend(data[pool+14*index:pool+14*index+14])
    struct.pack_into("<6i", result, 76, 1, new_frames, bones, new_pool, skeleton, len(result))
    (out / f"{rig}-reference.gla").write_bytes(result)
    print(f"{source.name}: sha256={hashlib.sha256(data).hexdigest()}, {bones} bones, reference={len(result)} bytes")
