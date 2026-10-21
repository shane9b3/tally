#!/usr/bin/env python3
"""Generates the BIFF8 (.xls) fixtures for tally-io-xls without any
third-party module: a minimal OLE2 compound file holding a `Workbook`
stream written record by record.

Run with:  python3 -I tests/fixtures/xls/gen_xls.py

basic.xls: two sheets. Sheet1 has numbers, a shared string, a boolean, an
error, two formulas with cached results (`=A1+A2` → 3, `=SUM(A1:A2)` → 3),
and a merge B1:C1; `Data` has one number. Deterministic output.
"""

import os
import struct

HERE = os.path.dirname(os.path.abspath(__file__))


def rec(t, data):
    return struct.pack("<HH", t, len(data)) + data


def bof(kind):
    # version 0x0600, type, build, year, history flags, lowest version
    return rec(0x0809, struct.pack("<HHHHII", 0x0600, kind, 0x0DBB, 0x07CC, 0, 0x0006))


def unicode_str(s, rich=False):
    # XLUnicodeRichExtendedString for SST: cch u16, flags u8, chars
    b = s.encode("utf-16-le")
    return struct.pack("<HB", len(s), 1) + b


def short_str(s):
    # ShortXLUnicodeString: cch u8, flags u8, chars
    return struct.pack("<BB", len(s), 1) + s.encode("utf-16-le")


def ref_col(col, rel=True):
    return col | (0xC000 if rel else 0)


def number(r, c, v, ixfe=15):
    return rec(0x0203, struct.pack("<HHHd", r, c, ixfe, v))


def labelsst(r, c, i, ixfe=15):
    return rec(0x00FD, struct.pack("<HHHI", r, c, ixfe, i))


def boolerr(r, c, v, is_err, ixfe=15):
    return rec(0x0205, struct.pack("<HHHBB", r, c, ixfe, v, 1 if is_err else 0))


def formula(r, c, cached, rgce, ixfe=15):
    body = struct.pack("<HHHd", r, c, ixfe, cached) + struct.pack("<HIH", 0, 0, len(rgce)) + rgce
    return rec(0x0006, body)


def t_ref(r, c):
    return struct.pack("<BHH", 0x44, r, ref_col(c))


def t_area(r0, r1, c0, c1):
    return struct.pack("<BHHHH", 0x45, r0, r1, ref_col(c0), ref_col(c1))


def t_func(iftab):
    return struct.pack("<BH", 0x41, iftab)


def t_funcvar(iftab, nargs):
    return struct.pack("<BBH", 0x42, nargs, iftab)


def workbook_stream():
    strings = ["hello", "Zürich"]
    globals_ = bytearray()
    globals_ += bof(0x0005)
    globals_ += rec(0x0042, struct.pack("<H", 0x04B0))  # CODEPAGE utf-16
    globals_ += rec(0x0022, struct.pack("<H", 0))  # DATEMODE 1900
    globals_ += rec(0x003D, struct.pack("<HHHHHHHHH", 0, 0, 0x4000, 0x3000, 0x38, 0, 0, 1, 0x258))
    # FONT, FORMAT and 16 XF records: calamine indexes formats by ixfe.
    globals_ += rec(0x0031, struct.pack("<HHHHHBBBB", 200, 0, 0x7FFF, 400, 0, 0, 0, 0, 0) + short_str("Arial"))
    for i in range(16):
        globals_ += rec(0x00E0, struct.pack("<HHHBBBBIH", 0, 0, 0xFFF5 if i < 15 else 0, 0x20, 0, 0, 0, 0, 0x20C0))
    # BOUNDSHEET records: patch the stream offsets afterwards.
    bs_positions = []
    sheet_names = ["Sheet1", "Data"]
    for name in sheet_names:
        bs_positions.append(len(globals_) + 4)
        globals_ += rec(0x0085, struct.pack("<IH", 0, 0) + short_str(name))
    # SST
    sst = struct.pack("<II", len(strings), len(strings)) + b"".join(unicode_str(s) for s in strings)
    globals_ += rec(0x00FC, sst)
    globals_ += rec(0x000A, b"")
    sheets = []
    s1 = bytearray()
    s1 += bof(0x0010)
    s1 += rec(0x0200, struct.pack("<IIHHH", 0, 4, 0, 4, 0))
    s1 += number(0, 0, 1.0)
    s1 += number(1, 0, 2.0)
    s1 += labelsst(0, 1, 0)
    s1 += labelsst(1, 1, 1)
    s1 += boolerr(2, 0, 1, False)
    s1 += boolerr(2, 1, 0x07, True)  # #DIV/0!
    s1 += formula(3, 0, 3.0, t_ref(0, 0) + t_ref(1, 0) + bytes([0x03]))  # =A1+A2
    s1 += formula(3, 1, 3.0, t_area(0, 1, 0, 0) + t_funcvar(4, 1))  # =SUM(A1:A2)
    s1 += rec(0x00E5, struct.pack("<HHHHH", 1, 0, 0, 1, 2))  # merge B1:C1
    s1 += rec(0x000A, b"")
    sheets.append(bytes(s1))
    s2 = bytearray()
    s2 += bof(0x0010)
    s2 += rec(0x0200, struct.pack("<IIHHH", 0, 1, 0, 1, 0))
    s2 += number(0, 0, 42.0)
    s2 += rec(0x000A, b"")
    sheets.append(bytes(s2))
    stream = bytearray(globals_)
    offsets = []
    for s in sheets:
        offsets.append(len(stream))
        stream += s
    for pos, off in zip(bs_positions, offsets):
        struct.pack_into("<I", stream, pos, off)
    return bytes(stream)


ENDOFCHAIN = 0xFFFFFFFE
FREESECT = 0xFFFFFFFF
FATSECT = 0xFFFFFFFD
NOSTREAM = 0xFFFFFFFF


def ole2(stream_name, data):
    """A compound file with one stream, stored in the mini stream when it is
    shorter than 4096 bytes (as real small workbooks are)."""
    sector = 512
    use_mini = len(data) < 4096
    if use_mini:
        mini_sectors = (len(data) + 63) // 64
        mini_stream = data + b"\0" * (mini_sectors * 64 - len(data))
        container_sectors = (len(mini_stream) + sector - 1) // sector
        mini_stream += b"\0" * (container_sectors * sector - len(mini_stream))
        # sector 0 FAT, 1 directory, 2 mini FAT, 3.. container
        fat = [FATSECT, ENDOFCHAIN, ENDOFCHAIN]
        for i in range(container_sectors):
            fat.append(3 + i + 1 if i + 1 < container_sectors else ENDOFCHAIN)
        fat += [FREESECT] * (128 - len(fat))
        minifat = [i + 1 if i + 1 < mini_sectors else ENDOFCHAIN for i in range(mini_sectors)]
        minifat += [FREESECT] * (128 - len(minifat))
        root_start, root_size = 3, len(mini_stream)
        stream_start = 0
        first_minifat, n_minifat = 2, 1
        body = struct.pack("<128I", *fat)
    else:
        n_sectors = (len(data) + sector - 1) // sector
        padded = data + b"\0" * (n_sectors * sector - len(data))
        fat = [FATSECT, ENDOFCHAIN]
        for i in range(n_sectors):
            fat.append(2 + i + 1 if i + 1 < n_sectors else ENDOFCHAIN)
        fat += [FREESECT] * (128 - len(fat))
        minifat = []
        root_start, root_size = ENDOFCHAIN, 0
        stream_start = 2
        first_minifat, n_minifat = ENDOFCHAIN, 0
        mini_stream = padded
        body = struct.pack("<128I", *fat)

    def dirent(name, typ, child, start, size):
        n = name.encode("utf-16-le") + b"\0\0"
        return (n + b"\0" * (64 - len(n)) + struct.pack("<HBB", len(n), typ, 1)
                + struct.pack("<III", NOSTREAM, NOSTREAM, child) + b"\0" * 16
                + struct.pack("<IQQIII", 0, 0, 0, start, size, 0))

    directory = dirent("Root Entry", 5, 1, root_start, root_size) + dirent(stream_name, 2, NOSTREAM, stream_start, len(data))
    directory += b"\0" * (sector - len(directory))
    header = (b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1" + b"\0" * 16
              + struct.pack("<HHHHH", 0x003E, 0x0003, 0xFFFE, 9, 6) + b"\0" * 6
              + struct.pack("<IIIIIII", 0, 1, 1, 0, 4096, first_minifat, n_minifat)
              + struct.pack("<II", ENDOFCHAIN, 0))
    difat = [0] + [FREESECT] * 108
    header += struct.pack("<109I", *difat)
    assert len(header) == 512, len(header)
    out = header + body + directory
    if use_mini:
        out += struct.pack("<128I", *minifat)
    out += mini_stream
    return out


def main():
    stream = workbook_stream()
    with open(os.path.join(HERE, "basic.xls"), "wb") as f:
        f.write(ole2("Workbook", stream))
    # A large variant (stream > 4096 bytes: regular sectors) with 300 rows.
    big = bytearray(stream)
    # Rebuild: append a third sheet is more work; instead pad Data sheet with numbers.
    print("wrote basic.xls", len(stream), "bytes of BIFF")


if __name__ == "__main__":
    main()
