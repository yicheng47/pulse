#!/usr/bin/env python3
"""Build and verify the stage 5 DSD64-over-PCM acceptance payload.

DSDIFF stores uncompressed DSD as byte-interleaved channels with the oldest bit
in each byte first. Two bytes per channel become one 24-bit DoP sample. Any
trailing DSD bits that do not fill a complete 16-bit DoP frame are dropped.

This is development tooling, not a Pulse decoder. It uses only the Python 3
standard library and the flac CLI.
"""

from __future__ import annotations

import argparse
import os
import struct
import subprocess
import sys
import tempfile
import wave
from dataclasses import dataclass
from pathlib import Path
from typing import BinaryIO


DEFAULT_SOURCE = Path.home() / "qobuz" / "04 - 暧昧.dff"
DEFAULT_OUTPUT = Path.home() / "qobuz" / "04 - 暧昧 (DoP).flac"
DEFAULT_FLAC = Path("flac")
DSD64_RATE = 2_822_400
DOP_RATE = 176_400
CHANNEL_IDS = (b"SLFT", b"SRGT")
CHANNEL_COUNT = len(CHANNEL_IDS)
DSD_BYTES_PER_DOP_FRAME = CHANNEL_COUNT * 2
WAV_BYTES_PER_FRAME = CHANNEL_COUNT * 3
PACK_FRAMES_PER_BLOCK = 262_144
VERIFY_SECONDS = 10


class PayloadError(Exception):
    pass


@dataclass(frozen=True)
class DffInfo:
    sample_rate: int
    channel_ids: tuple[bytes, ...]
    compression_type: bytes
    data_offset: int
    data_size: int

    @property
    def dsd_bytes_per_channel(self) -> int:
        return self.data_size // len(self.channel_ids)

    @property
    def dop_frames(self) -> int:
        return self.dsd_bytes_per_channel // 2

    @property
    def dropped_bits_per_channel(self) -> int:
        return (self.dsd_bytes_per_channel % 2) * 8

    @property
    def dsd_duration(self) -> float:
        return self.dsd_bytes_per_channel * 8 / self.sample_rate


def read_exact(stream: BinaryIO, size: int, description: str) -> bytes:
    data = stream.read(size)
    if len(data) != size:
        raise PayloadError(f"truncated {description}")
    return data


def read_chunk_header(stream: BinaryIO, limit: int, parent: str) -> tuple[bytes, int, int, int]:
    header_offset = stream.tell()
    if header_offset + 12 > limit:
        raise PayloadError(f"truncated chunk header in {parent}")

    chunk_id = read_exact(stream, 4, f"{parent} chunk ID")
    chunk_size = struct.unpack(">Q", read_exact(stream, 8, f"{chunk_id!r} size"))[0]
    data_offset = stream.tell()
    padded_end = data_offset + chunk_size + (chunk_size & 1)
    if padded_end > limit:
        raise PayloadError(f"{chunk_id!r} chunk exceeds {parent}")
    return chunk_id, chunk_size, data_offset, padded_end


def parse_property_chunk(stream: BinaryIO, data_offset: int, size: int) -> dict[bytes, object]:
    end = data_offset + size
    stream.seek(data_offset)
    if size < 4 or read_exact(stream, 4, "PROP type") != b"SND ":
        raise PayloadError("PROP chunk is not sound properties")

    properties: dict[bytes, object] = {}
    while stream.tell() < end:
        chunk_id, chunk_size, chunk_data, padded_end = read_chunk_header(stream, end, "PROP")
        if chunk_id in properties:
            raise PayloadError(f"duplicate {chunk_id!r} chunk in PROP")

        if chunk_id == b"FS  ":
            if chunk_size != 4:
                raise PayloadError("FS chunk must contain one 32-bit sample rate")
            properties[chunk_id] = struct.unpack(">I", read_exact(stream, 4, "sample rate"))[0]
        elif chunk_id == b"CHNL":
            if chunk_size < 2:
                raise PayloadError("CHNL chunk is too short")
            channel_count = struct.unpack(">H", read_exact(stream, 2, "channel count"))[0]
            if chunk_size != 2 + channel_count * 4:
                raise PayloadError("CHNL chunk size does not match its channel count")
            properties[chunk_id] = tuple(
                read_exact(stream, 4, "channel ID") for _ in range(channel_count)
            )
        elif chunk_id == b"CMPR":
            if chunk_size < 5:
                raise PayloadError("CMPR chunk is too short")
            compression_type = read_exact(stream, 4, "compression type")
            name_length = read_exact(stream, 1, "compression name length")[0]
            if name_length > chunk_size - 5:
                raise PayloadError("CMPR name exceeds its chunk")
            properties[chunk_id] = compression_type

        stream.seek(padded_end)

    return properties


def parse_dff(path: Path) -> DffInfo:
    file_size = path.stat().st_size
    with path.open("rb") as stream:
        if read_exact(stream, 4, "file ID") != b"FRM8":
            raise PayloadError("source is not a DSDIFF FRM8 container")
        form_size = struct.unpack(">Q", read_exact(stream, 8, "FRM8 size"))[0]
        form_end = 12 + form_size
        if form_end != file_size:
            raise PayloadError(
                f"FRM8 size is {form_size} bytes but file size implies {file_size - 12}"
            )
        if form_size < 4 or read_exact(stream, 4, "FRM8 type") != b"DSD ":
            raise PayloadError("FRM8 container is not DSD")

        properties: dict[bytes, object] | None = None
        data_offset: int | None = None
        data_size: int | None = None
        chunk_index = 0

        while stream.tell() < form_end:
            chunk_id, chunk_size, chunk_data, padded_end = read_chunk_header(
                stream, form_end, "FRM8"
            )
            if chunk_index == 0 and chunk_id != b"FVER":
                raise PayloadError("FVER must be the first FRM8 chunk")
            chunk_index += 1

            if chunk_id == b"FVER":
                if chunk_size != 4:
                    raise PayloadError("FVER chunk must contain one 32-bit version")
                read_exact(stream, 4, "DSDIFF version")
            elif chunk_id == b"PROP":
                if properties is not None:
                    raise PayloadError("duplicate PROP chunk")
                if data_offset is not None:
                    raise PayloadError("PROP chunk must precede sound data")
                properties = parse_property_chunk(stream, chunk_data, chunk_size)
            elif chunk_id == b"DSD ":
                if properties is None:
                    raise PayloadError("DSD sound data precedes PROP")
                if data_offset is not None:
                    raise PayloadError("duplicate DSD sound data chunk")
                data_offset = chunk_data
                data_size = chunk_size
            elif chunk_id == b"DST ":
                raise PayloadError("DST-compressed DSD is not supported")

            stream.seek(padded_end)

    if properties is None:
        raise PayloadError("missing PROP chunk")
    if data_offset is None or data_size is None:
        raise PayloadError("missing uncompressed DSD sound data chunk")

    sample_rate = properties.get(b"FS  ")
    channel_ids = properties.get(b"CHNL")
    compression_type = properties.get(b"CMPR")
    if not isinstance(sample_rate, int):
        raise PayloadError("missing FS chunk")
    if not isinstance(channel_ids, tuple):
        raise PayloadError("missing CHNL chunk")
    if not isinstance(compression_type, bytes):
        raise PayloadError("missing CMPR chunk")
    if sample_rate != DSD64_RATE:
        raise PayloadError(f"expected DSD64 at {DSD64_RATE} Hz, found {sample_rate} Hz")
    if channel_ids != CHANNEL_IDS:
        labels = ", ".join(repr(channel_id) for channel_id in channel_ids)
        raise PayloadError(f"expected stereo channels {CHANNEL_IDS!r}, found {labels}")
    if compression_type != b"DSD ":
        raise PayloadError(f"expected uncompressed DSD, found {compression_type!r}")
    if data_size % CHANNEL_COUNT:
        raise PayloadError("DSD sound data does not contain complete stereo clustered frames")

    return DffInfo(
        sample_rate=sample_rate,
        channel_ids=channel_ids,
        compression_type=compression_type,
        data_offset=data_offset,
        data_size=data_size,
    )


def marker_bytes(start_frame: int, frame_count: int) -> bytes:
    pair = b"\x05\xfa" if start_frame % 2 == 0 else b"\xfa\x05"
    return (pair * ((frame_count + 1) // 2))[:frame_count]


def pack_wav(source: Path, wav_path: Path, info: DffInfo) -> None:
    if info.dop_frames * WAV_BYTES_PER_FRAME + 44 > 0xFFFFFFFF:
        raise PayloadError("intermediate WAV would exceed the 4 GiB RIFF limit")

    complete_data_size = info.dop_frames * DSD_BYTES_PER_DOP_FRAME
    frame_index = 0

    with source.open("rb") as dff, wave.open(str(wav_path), "wb") as wav:
        dff.seek(info.data_offset)
        wav.setnchannels(CHANNEL_COUNT)
        wav.setsampwidth(3)
        wav.setframerate(DOP_RATE)

        remaining = complete_data_size
        while remaining:
            block_size = min(remaining, PACK_FRAMES_PER_BLOCK * DSD_BYTES_PER_DOP_FRAME)
            dsd = read_exact(dff, block_size, "DSD sound data")
            frame_count = block_size // DSD_BYTES_PER_DOP_FRAME
            markers = marker_bytes(frame_index, frame_count)
            packed = bytearray(frame_count * WAV_BYTES_PER_FRAME)

            # DoP is marker:23..16, earlier DSD:15..8, later DSD:7..0;
            # little-endian WAV stores each sample as [later, earlier, marker].
            packed[0::6] = dsd[2::4]
            packed[1::6] = dsd[0::4]
            packed[2::6] = markers
            packed[3::6] = dsd[3::4]
            packed[4::6] = dsd[1::4]
            packed[5::6] = markers

            wav.writeframesraw(packed)
            remaining -= block_size
            frame_index += frame_count

    if frame_index != info.dop_frames:
        raise PayloadError(f"packed {frame_index} frames, expected {info.dop_frames}")


def encode_flac(flac: Path, wav_path: Path, output: Path) -> None:
    subprocess.run(
        [
            str(flac),
            "--best",
            "--verify",
            "--force",
            f"--output-name={output}",
            str(wav_path),
        ],
        check=True,
    )


def check_payload_window(
    wav: wave.Wave_read,
    source: Path,
    info: DffInfo,
    start_frame: int,
    frame_count: int,
    label: str,
) -> None:
    wav.setpos(start_frame)
    frames = wav.readframes(frame_count)
    expected_size = frame_count * WAV_BYTES_PER_FRAME
    if len(frames) != expected_size:
        raise PayloadError(f"decoded WAV is truncated in the {label} payload window")

    with source.open("rb") as dff:
        dff.seek(info.data_offset + start_frame * DSD_BYTES_PER_DOP_FRAME)
        dsd = read_exact(dff, frame_count * DSD_BYTES_PER_DOP_FRAME, f"{label} DSD data")

    markers = marker_bytes(start_frame, frame_count)
    expected = bytearray(expected_size)
    expected[0::6] = dsd[2::4]
    expected[1::6] = dsd[0::4]
    expected[2::6] = markers
    expected[3::6] = dsd[3::4]
    expected[4::6] = dsd[1::4]
    expected[5::6] = markers
    if frames != expected:
        mismatch = next(
            index
            for index, (found, wanted) in enumerate(zip(frames, expected))
            if found != wanted
        )
        lane_names = (
            "left low",
            "left high",
            "left marker",
            "right low",
            "right high",
            "right marker",
        )
        absolute_frame = start_frame + mismatch // WAV_BYTES_PER_FRAME
        raise PayloadError(
            f"{label} {lane_names[mismatch % WAV_BYTES_PER_FRAME]} mismatch at "
            f"frame {absolute_frame}: found 0x{frames[mismatch]:02X}, "
            f"expected 0x{expected[mismatch]:02X}"
        )


def verify_flac(flac: Path, source: Path, output: Path, info: DffInfo) -> tuple[float, int]:
    with tempfile.TemporaryDirectory(prefix="pulse-dop-verify-") as temp_dir:
        decoded_wav = Path(temp_dir) / "decoded.wav"
        subprocess.run(
            [
                str(flac),
                "--decode",
                "--force-legacy-wave-format",
                "--silent",
                "--force",
                f"--output-name={decoded_wav}",
                str(output),
            ],
            check=True,
        )

        with wave.open(str(decoded_wav), "rb") as wav:
            channels = wav.getnchannels()
            sample_width = wav.getsampwidth()
            sample_rate = wav.getframerate()
            frame_count = wav.getnframes()
            compression = wav.getcomptype()
            if (channels, sample_width, sample_rate, compression) != (
                CHANNEL_COUNT,
                3,
                DOP_RATE,
                "NONE",
            ):
                raise PayloadError(
                    "decoded format is not 176.4 kHz / 24-bit stereo PCM: "
                    f"channels={channels}, width={sample_width * 8}, "
                    f"rate={sample_rate}, compression={compression}"
                )
            if frame_count != info.dop_frames:
                raise PayloadError(
                    f"decoded frame count is {frame_count}, expected {info.dop_frames}"
                )

            window_frames = VERIFY_SECONDS * DOP_RATE
            if frame_count < window_frames * 2:
                raise PayloadError("payload is too short for first and last 10-second checks")
            check_payload_window(wav, source, info, 0, window_frames, "first 10-second")
            check_payload_window(
                wav,
                source,
                info,
                frame_count - window_frames,
                window_frames,
                "last 10-second",
            )

    duration = frame_count / DOP_RATE
    if abs(duration - info.dsd_duration) > 1 / DOP_RATE:
        raise PayloadError(
            f"decoded duration {duration:.6f}s does not match DSD duration "
            f"{info.dsd_duration:.6f}s"
        )
    return duration, frame_count


def format_size(size: int) -> str:
    return f"{size / (1024 * 1024):.2f} MiB ({size} bytes)"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=DEFAULT_SOURCE)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--flac", type=Path, default=DEFAULT_FLAC)
    parser.add_argument(
        "--verify",
        action="store_true",
        help="verify the existing output without rebuilding it",
    )
    return parser.parse_args()


def run() -> None:
    args = parse_args()
    info = parse_dff(args.source)
    print(
        f"Source: DSD64 stereo, {info.sample_rate} Hz, "
        f"{info.data_size} DSD bytes, {info.dop_frames} DoP frames"
    )
    if info.dropped_bits_per_channel:
        print(f"Dropped trailing bits per channel: {info.dropped_bits_per_channel}")

    if not args.verify:
        file_descriptor, wav_name = tempfile.mkstemp(
            prefix=f"{args.output.stem}-", suffix=".wav", dir=args.output.parent
        )
        os.close(file_descriptor)
        wav_path = Path(wav_name)
        try:
            print(f"Packing intermediate WAV: {wav_path}", flush=True)
            pack_wav(args.source, wav_path, info)
            print(f"Encoding lossless FLAC: {args.output}", flush=True)
            encode_flac(args.flac, wav_path, args.output)
        finally:
            wav_path.unlink(missing_ok=True)
        print("Removed intermediate WAV after successful encode")

    print("Decoding FLAC for offline structural verification", flush=True)
    duration, frame_count = verify_flac(args.flac, args.source, args.output, info)
    output_size = args.output.stat().st_size
    print(f"Output: {args.output}")
    print(f"Size: {format_size(output_size)}")
    print(f"Duration: {duration:.6f} seconds ({frame_count} frames at {DOP_RATE} Hz)")
    print(
        "Payload: PASS (source-exact DoP frames and alternating markers, "
        "both channels, first and last 10 seconds)"
    )
    print("VERDICT: PASS")


def main() -> int:
    try:
        run()
    except (OSError, PayloadError, subprocess.CalledProcessError, wave.Error) as error:
        print(f"VERDICT: FAIL - {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
