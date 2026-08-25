"""Minimal read/patch access to Windows registry hive files (regf).

Only what offline Win7 configuration needs: walk keys, read values, and rewrite a
value's name or data *in place* when the replacement fits the existing cell. In-place
edits keep every cell size and every offset in the hive unchanged, so no allocator,
free-list or parent-pointer bookkeeping is involved.
"""

import os
import struct

BASE = 0x1000


class Hive:
    def __init__(self, path, mutable=False):
        self.path = path
        self.data = bytearray(open(path, "rb").read())
        self.mutable = mutable
        if self.data[0:4] != b"regf":
            raise ValueError(f"{path} is not a registry hive (no regf signature)")
        self.root_off = struct.unpack_from("<I", self.data, 0x24)[0]

    # ---- cell access -------------------------------------------------
    def cell(self, off):
        """Return (start, size) of the payload of the cell at hive offset `off`."""
        p = BASE + off
        size = struct.unpack_from("<i", self.data, p)[0]
        return p + 4, abs(size) - 4

    def u16(self, p):
        return struct.unpack_from("<H", self.data, p)[0]

    def u32(self, p):
        return struct.unpack_from("<I", self.data, p)[0]

    # ---- key nodes ---------------------------------------------------
    def key_name(self, nk):
        flags = self.u16(nk + 2)
        nlen = self.u16(nk + 0x48)
        raw = bytes(self.data[nk + 0x4C : nk + 0x4C + nlen])
        return raw.decode("latin-1") if flags & 0x20 else raw.decode("utf-16-le")

    def subkeys(self, nk):
        count = self.u32(nk + 0x14)
        lst = self.u32(nk + 0x1C)
        if count == 0 or lst in (0, 0xFFFFFFFF):
            return []
        return self._collect_list(lst)

    def _collect_list(self, off):
        p, _ = self.cell(off)
        sig = bytes(self.data[p : p + 2])
        n = self.u16(p + 2)
        out = []
        if sig in (b"lf", b"lh"):
            for i in range(n):
                out.append(self.u32(p + 4 + i * 8))
        elif sig == b"li":
            for i in range(n):
                out.append(self.u32(p + 4 + i * 4))
        elif sig == b"ri":
            for i in range(n):
                out.extend(self._collect_list(self.u32(p + 4 + i * 4)))
        return [self.cell(o)[0] for o in out]

    def find(self, path, nk=None):
        """Resolve a backslash-separated key path, case-insensitively."""
        node = nk if nk is not None else self.cell(self.root_off)[0]
        for part in [p for p in path.split("\\") if p]:
            for sub in self.subkeys(node):
                if self.key_name(sub).lower() == part.lower():
                    node = sub
                    break
            else:
                return None
        return node

    # ---- values ------------------------------------------------------
    def values(self, nk):
        count = self.u32(nk + 0x24)
        lst = self.u32(nk + 0x28)
        if count == 0 or lst in (0, 0xFFFFFFFF):
            return []
        p, _ = self.cell(lst)
        return [self.cell(self.u32(p + i * 4))[0] for i in range(count)]

    def value_name(self, vk):
        nlen = self.u16(vk + 2)
        if nlen == 0:
            return "(default)"
        raw = bytes(self.data[vk + 0x14 : vk + 0x14 + nlen])
        return raw.decode("latin-1") if self.u16(vk + 0x10) & 1 else raw.decode("utf-16-le")

    def value_raw(self, vk):
        size = self.u32(vk + 4)
        off = self.u32(vk + 8)
        if size & 0x80000000:
            n = size & 0xFFFF
            return bytes(self.data[vk + 8 : vk + 8 + n])
        p, _ = self.cell(off)
        return bytes(self.data[p : p + size])

    def value_type(self, vk):
        return self.u32(vk + 0x0C)

    def value(self, nk, name):
        for vk in self.values(nk):
            if self.value_name(vk).lower() == name.lower():
                return vk
        return None

    def value_str(self, vk):
        raw = self.value_raw(vk)
        t = self.value_type(vk)
        if t in (1, 2, 7):
            return raw.decode("utf-16-le", "replace").rstrip("\x00")
        if t == 4:
            return str(struct.unpack_from("<I", raw + b"\0\0\0\0", 0)[0])
        return raw.hex()

    def _require_mutable(self):
        # Not an assert: `python3 -O` strips those, and every caller below writes
        # into the hive.
        if not self.mutable:
            raise RuntimeError("hive opened read-only; pass mutable=True")

    # ---- in-place mutation ------------------------------------------
    def set_value_name(self, vk, new_name):
        """Rename a value, reusing its existing name field. ASCII names only."""
        self._require_mutable()
        old_len = self.u16(vk + 2)
        enc = new_name.encode("latin-1")
        if len(enc) > old_len:
            # A slice assignment with an over-long right-hand side would *grow*
            # the bytearray and shift every cell after it — silent corruption.
            raise ValueError(f"name {new_name!r} longer than its field allows ({old_len})")
        struct.pack_into("<H", self.data, vk + 2, len(enc))
        struct.pack_into("<H", self.data, vk + 0x10, self.u16(vk + 0x10) | 1)
        self.data[vk + 0x14 : vk + 0x14 + old_len] = enc.ljust(old_len, b"\0")

    def set_value_data(self, vk, data, vtype):
        """Rewrite a value's data, reusing its cell when the payload fits.

        When it does not fit — including growing a small inline value into a real
        cell — a new cell is allocated and the old one released. Offsets of other
        cells are never disturbed, so this is safe to do on a live hive.
        """
        self._require_mutable()
        struct.pack_into("<I", self.data, vk + 0x0C, vtype)

        # Four bytes or fewer live inline in the vk itself.
        if len(data) <= 4:
            old_size = self.u32(vk + 4)
            if not (old_size & 0x80000000) and old_size != 0:
                self.free(self.u32(vk + 8))  # was out-of-line; release the cell
            struct.pack_into("<I", self.data, vk + 4, 0x80000000 | len(data))
            self.data[vk + 8 : vk + 12] = data.ljust(4, b"\0")
            return

        old_size = self.u32(vk + 4)
        inline = bool(old_size & 0x80000000)
        if not inline and old_size != 0:
            _, cap = self.cell(self.u32(vk + 8))
            if len(data) <= cap:
                p, _ = self.cell(self.u32(vk + 8))
                struct.pack_into("<I", self.data, vk + 4, len(data))
                self.data[p : p + len(data)] = data
                return
            self.free(self.u32(vk + 8))

        off = self.alloc(len(data))
        p, cap = self.cell(off)
        if len(data) > cap:
            raise RuntimeError(f"allocated cell too small: {cap}B for {len(data)}B")
        self.data[p : p + len(data)] = data
        struct.pack_into("<I", self.data, vk + 4, len(data))
        struct.pack_into("<I", self.data, vk + 8, off)

    # ---- allocation --------------------------------------------------
    def _hbins(self):
        p = BASE
        while p + 32 <= len(self.data) and self.data[p : p + 4] == b"hbin":
            size = self.u32(p + 8)
            yield p, size
            p += size

    def alloc(self, payload_bytes):
        """Carve a cell out of the hive's free space; returns its hive offset.

        Cells are 8-byte multiples whose leading i32 is the total size, negative
        when allocated. A free cell larger than the request is split, and the
        remainder stays free.
        """
        self._require_mutable()
        need = max(8, (payload_bytes + 4 + 7) & ~7)
        for start, size in self._hbins():
            q = start + 32
            while q < start + size:
                sz = struct.unpack_from("<i", self.data, q)[0]
                if sz == 0:
                    break
                if sz > 0 and sz >= need:
                    rest = sz - need
                    if rest >= 8:
                        struct.pack_into("<i", self.data, q + need, rest)
                    else:
                        need = sz
                    struct.pack_into("<i", self.data, q, -need)
                    off = q - BASE
                    self.data[q + 4 : q + need] = b"\0" * (need - 4)
                    return off
                q += abs(sz)
        raise RuntimeError("no free cell large enough")

    def free(self, off):
        p = BASE + off
        size = struct.unpack_from("<i", self.data, p)[0]
        struct.pack_into("<i", self.data, p, abs(size))

    def add_value(self, nk, name, vtype, data):
        """Append a new value to a key, allocating the vk, its data and a wider value list."""
        self._require_mutable()
        if self.value(nk, name) is not None:
            raise ValueError(f"{name} already exists")
        enc = name.encode("latin-1")

        if len(data) <= 4:
            data_size, data_off = 0x80000000 | len(data), int.from_bytes(data.ljust(4, b"\0"), "little")
        else:
            cell = self.alloc(len(data))
            p, _ = self.cell(cell)
            self.data[p : p + len(data)] = data
            data_size, data_off = len(data), cell

        vk_off = self.alloc(0x14 + len(enc))
        vk, _ = self.cell(vk_off)
        self.data[vk : vk + 2] = b"vk"
        struct.pack_into("<H", self.data, vk + 2, len(enc))
        struct.pack_into("<I", self.data, vk + 4, data_size)
        struct.pack_into("<I", self.data, vk + 8, data_off)
        struct.pack_into("<I", self.data, vk + 0x0C, vtype)
        struct.pack_into("<H", self.data, vk + 0x10, 1)  # ASCII name
        self.data[vk + 0x14 : vk + 0x14 + len(enc)] = enc

        count = self.u32(nk + 0x24)
        old_list = self.u32(nk + 0x28)
        old = []
        if count:
            lp, _ = self.cell(old_list)
            old = [self.u32(lp + i * 4) for i in range(count)]
        new_list = self.alloc((count + 1) * 4)
        lp, _ = self.cell(new_list)
        for i, o in enumerate(old + [vk_off]):
            struct.pack_into("<I", self.data, lp + i * 4, o)
        struct.pack_into("<I", self.data, nk + 0x24, count + 1)
        struct.pack_into("<I", self.data, nk + 0x28, new_list)
        if count:
            self.free(old_list)

        if len(enc) > self.u32(nk + 0x3C):
            struct.pack_into("<I", self.data, nk + 0x3C, len(enc))
        if len(data) > self.u32(nk + 0x40):
            struct.pack_into("<I", self.data, nk + 0x40, len(data))
        return vk

    def dirty(self):
        """True when the hive's two sequence numbers disagree.

        Windows writes seq1 before a change and seq2 after. If they differ the
        hive was not cleanly unmounted, and on the next boot Windows replays
        `.LOG1`/`.LOG2` over it — which silently reverts anything edited here.
        """
        return self.u32(4) != self.u32(8)

    def log_files(self):
        """Sibling transaction logs, which can undo an edit if they are replayed."""
        return [
            p
            for p in (self.path + ".LOG", self.path + ".LOG1", self.path + ".LOG2")
            if os.path.exists(p)
        ]

    def save(self, path=None):
        """Write the hive back atomically.

        The write goes to a sibling temp file which is then renamed over the
        target, so an interrupted save cannot leave a half-written SAM or SYSTEM
        behind — losing one of those costs a reinstall.
        """
        self._require_mutable()
        recompute_checksum(self.data)
        dest = path or self.path
        tmp = dest + ".tmp-hive-write"
        with open(tmp, "wb") as f:
            f.write(self.data)
            f.flush()
            os.fsync(f.fileno())
        os.replace(tmp, dest)


def recompute_checksum(data):
    """The base block's XOR checksum covers the first 508 bytes."""
    csum = 0
    for i in range(0, 508, 4):
        csum ^= struct.unpack_from("<I", data, i)[0]
    if csum == 0:
        csum = 1
    elif csum == 0xFFFFFFFF:
        csum = 0xFFFFFFFE
    struct.pack_into("<I", data, 508, csum)


def dump(h, path, limit=60):
    nk = h.find(path)
    if nk is None:
        print(f"  [missing] {path}")
        return None
    print(f"  {path}")
    for vk in h.values(nk)[:limit]:
        print(f"    {h.value_name(vk):<34} type={h.value_type(vk):<3} {h.value_str(vk)[:70]!r}")
    return nk
