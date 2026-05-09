# Napkin Runbook

## Curation Rules

- Re-prioritize on every read.
- Max 10 items per category.
- Each item includes date + "Do instead".

## Execution & Validation

1. **[2026-05-08] Validar com build real do Tauri**
   Do instead: `npm run tauri build` após mudanças em Rust ou `tauri.conf.json`.

2. **[2026-05-09] Build portátil Windows (sem instalador)**
   Do instead: `npm run tauri:win-portable` → `dist-portable/` (pasta + `.zip` com `VideoDivider.exe`).

3. **[2026-05-08] FFmpeg não vai no bundle**
   Do instead: lembrar usuários (README) de instalar FFmpeg e ter `ffmpeg`/`ffprobe` no PATH.

## Stack Guardrails

1. **[2026-05-08] Divisão por GB é heurística com `-c copy`**
   Do instead: deixar claro na UI que partes podem passar um pouco do teto por causa de GOP/keyframes.

2. **[2026-05-09] Aba MP4 → MKV: remux com `-c copy`**
   Do instead: eventos `convert-*`; saída `stem.mkv`; não sobrescrever se já existir; `split_cancel` encerra também a conversão.

## Shell & Paths (Windows)

1. **`which`/PATH no Tauri**
   Do instead: testar `ffmpeg_status` no app depois de alterar PATH; reiniciar o app para pegar PATH novo.

2. **Spawn de FFmpeg/ffprobe/taskkill não deve piscar janela de console**
   Do instead: em novos `Command` filhos no Windows, reutilizar `suppress_child_console` (`CREATE_NO_WINDOW` / `0x08000000`) como em `lib.rs`.
