#!/usr/bin/env python3
"""Generate the tiny DSF/DFF parser fixtures and byte-exact DoP expectations."""

from __future__ import annotations

import struct
import tempfile
import wave
from pathlib import Path

from dop_pack import pack_wav, parse_dff


FIXTURES = Path(__file__).resolve().parents[1] / "crates/pulse-engine/tests/fixtures"


def dff_chunk(chunk_id: bytes, data: bytes) -> bytes:
    return chunk_id + struct.pack(">Q", len(data)) + data + (b"\0" if len(data) & 1 else b"")


def build_dff(compression: bytes, data: bytes) -> bytes:
    properties = b"SND "
    properties += dff_chunk(b"FS  ", struct.pack(">I", 2_822_400))
    properties += dff_chunk(b"CHNL", struct.pack(">H", 2) + b"SLFTSRGT")
    name = b"not compressed" if compression == b"DSD " else b"DST"
    properties += dff_chunk(b"CMPR", compression + bytes([len(name)]) + name)
    form = b"DSD "
    form += dff_chunk(b"FVER", struct.pack(">I", 0x01050000))
    form += dff_chunk(b"PROP", properties)
    form += dff_chunk(b"DSD ", data)
    return b"FRM8" + struct.pack(">Q", len(form)) + form


def write_dsf() -> None:
    channel_0 = bytes([0x01, 0x02, 0x04, 0x08]) + bytes(4092)
    channel_1 = bytes([0x80, 0x40, 0x20, 0x10]) + bytes(4092)
    data = channel_0 + channel_1
    fmt = struct.pack("<IIIIIIQII", 1, 0, 2, 2, 2_822_400, 1, 32, 4096, 0)
    file_size = 28 + 12 + len(fmt) + 12 + len(data)
    dsf = b"DSD " + struct.pack("<QQQ", 28, file_size, 0)
    dsf += b"fmt " + struct.pack("<Q", 52) + fmt
    dsf += b"data" + struct.pack("<Q", 12 + len(data)) + data
    FIXTURES.joinpath("dsd-bit-reversal.dsf").write_bytes(dsf)
    FIXTURES.joinpath("dsd-bit-reversal.dop").write_bytes(
        bytes([0x40, 0x80, 0x05, 0x02, 0x01, 0x05, 0x10, 0x20, 0xFA, 0x08, 0x04, 0xFA])
    )


def write_dff() -> None:
    data = bytes(range(256)) * 3 + bytes(range(32))
    source = FIXTURES / "dsd-interleave.dff"
    source.write_bytes(build_dff(b"DSD ", data))
    with tempfile.NamedTemporaryFile(suffix=".wav") as output:
        pack_wav(source, Path(output.name), parse_dff(source))
        with wave.open(output.name, "rb") as wav:
            FIXTURES.joinpath("dsd-interleave.dop").write_bytes(
                wav.readframes(wav.getnframes())
            )
    FIXTURES.joinpath("dst-refusal.dff").write_bytes(build_dff(b"DST ", b""))


if __name__ == "__main__":
    write_dsf()
    write_dff()
