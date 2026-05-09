# VideoDivider

Desktop tool to **split movies or series into multiple files** under a maximum size you choose (in GB).

## Why this app exists

This project targets **older TVs** (or USB media players) where a **flash drive or external disk is only recognized when formatted as FAT32**. On that filesystem, **each file must stay under about 4 GB** — large MKV/MP4 files cannot be played from USB unless you split them. VideoDivider uses **FFmpeg** in **stream-copy** mode (`-c copy`: fast, no re-encoding). You set a per-part size cap in GB and get numbered files you can copy to USB and watch on the TV.

**Note:** actual part sizes may vary slightly (cuts on *keyframes*). To stay safely under the FAT32 limit, use a cap **a bit below 4 GB** (e.g. 3.7 or 3.8 GB) instead of exactly 4.0 GB.

## MP4 → MKV (remux, optional)

The app has a separate **MP4 → MKV** mode: it **remuxes** `.mp4` / `.m4v` into **`.mkv`** using FFmpeg **stream copy** (`-c copy` again — fast, **no re-encoding**). Video stays in the same codec (e.g. **H.264** stays H.264); only the **container** changes to Matroska. Output is **`basename.mkv`** in the folder you choose; if that file already exists, the conversion is **refused** (no overwrite). Some streams may fail to copy into MKV depending on the file; then FFmpeg will error and any partial output is removed.

## Requirement: FFmpeg

Install [FFmpeg](https://ffmpeg.org/download.html) and ensure **`ffmpeg`** and **`ffprobe`** are on your **PATH**. The VideoDivider binary **does not bundle** FFmpeg; it only invokes it.

## External `.srt` subtitles (optional)

After splitting the video, you can point to an `.srt` with the **same base name** as the video (`movie.mkv` + `movie.srt`). The app writes `movie_part_001.srt`, `movie_part_002.srt`, … with **recalculated** timing for each part. **UTF-8** and **Windows-1252** encodings are supported.

Use an output folder without leftover parts from earlier runs that share the same filename prefix, so old files are not picked up by mistake.

## Development

```bash
npm install
npm run tauri dev
```

## Build

```bash
npm run tauri build
```

### Portable Windows (folder + ZIP, no installer)

Produces `dist-portable/VideoDivider-<version>-windows-x64-portable/` with the `.exe` and a matching `.zip`, after a **release** Tauri build:

```bash
npm run tauri:win-portable
```

You still need **FFmpeg** on the machine **PATH** when running the portable executable.

## License

**Source code and binaries in this repository (VideoDivider)** are governed by [`LICENSE`](LICENSE): **proprietary use, all rights reserved**, with **no** permission granted to third parties (the usual wording for private, closed-source software).

**FFmpeg** (and other third-party components) remains under **their** licenses; if you distribute builds that bundle FFmpeg binaries, comply with that project’s legal requirements.
