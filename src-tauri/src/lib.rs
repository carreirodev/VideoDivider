mod srt;

use regex::Regex;
use serde::Serialize;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use tauri::{AppHandle, Emitter, State};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FfmpegStatus {
    pub available: bool,
    pub ffmpeg_path: Option<String>,
    pub ffprobe_path: Option<String>,
    pub message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    pub duration_sec: f64,
    pub size_bytes: u64,
    pub extension: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SplitProgressPayload {
    pub line: String,
    pub ratio: Option<f64>,
}

pub struct SplitState {
    current_pid: Mutex<Option<u32>>,
}

/// No Windows, processos console (ffmpeg, ffprobe, taskkill) não abrem janela de terminal.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn suppress_child_console(cmd: &mut Command) {
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
}

fn resolve_bin(name: &str) -> Option<PathBuf> {
    which::which(name).ok()
}

#[tauri::command]
fn ffmpeg_status() -> FfmpegStatus {
    let ffmpeg = resolve_bin("ffmpeg");
    let ffprobe = resolve_bin("ffprobe");
    let available = ffmpeg.is_some() && ffprobe.is_some();
    let message = if available {
        "FFmpeg e FFprobe encontrados no PATH.".to_string()
    } else {
        "Instale o FFmpeg e adicione ffmpeg e ffprobe ao PATH do sistema.".to_string()
    };
    FfmpegStatus {
        available,
        ffmpeg_path: ffmpeg.map(|p| p.to_string_lossy().into_owned()),
        ffprobe_path: ffprobe.map(|p| p.to_string_lossy().into_owned()),
        message,
    }
}

fn probe_duration_only(path: &str) -> Result<f64, String> {
    let ffprobe = resolve_bin("ffprobe").ok_or("ffprobe não encontrado no PATH.")?;
    let mut cmd = Command::new(&ffprobe);
    cmd.args([
        "-v",
        "error",
        "-show_entries",
        "format=duration",
        "-of",
        "default=noprint_wrappers=1:nokey=1",
        path,
    ]);
    suppress_child_console(&mut cmd);
    let out = cmd
        .output()
        .map_err(|e| format!("Falha ao executar ffprobe: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).to_string());
    }
    let dur_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if dur_str.is_empty() || dur_str == "N/A" {
        return Err(
            "Não foi possível obter a duração do arquivo (formato ou fluxo incompatível?).".into(),
        );
    }
    dur_str
        .parse()
        .map_err(|_| format!("Duração inválida do ffprobe: {dur_str:?}"))
}

#[tauri::command]
fn probe_file(path: String) -> Result<ProbeResult, String> {
    let p = Path::new(&path);
    if !p.is_file() {
        return Err("Arquivo não encontrado.".into());
    }
    let meta = fs::metadata(&path).map_err(|e| e.to_string())?;
    let duration_sec = probe_duration_only(&path)?;
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mkv")
        .to_lowercase();
    Ok(ProbeResult {
        duration_sec,
        size_bytes: meta.len(),
        extension: ext,
    })
}

fn segment_format_for_ext(ext: &str) -> &'static str {
    match ext {
        "mkv" | "mka" | "mks" => "matroska",
        "mp4" | "m4v" => "mp4",
        "mov" => "mov",
        "webm" => "webm",
        "avi" => "avi",
        "ts" | "mts" | "m2ts" => "mpegts",
        _ => "matroska",
    }
}

fn estimate_segment_seconds(max_part_bytes: u64, size_bytes: u64, duration_sec: f64) -> f64 {
    if duration_sec <= 0.0 || size_bytes == 0 {
        return 60.0;
    }
    let bytes_per_sec = size_bytes as f64 / duration_sec;
    if bytes_per_sec <= 0.0 {
        return 60.0;
    }
    let raw = max_part_bytes as f64 / bytes_per_sec * 0.85;
    raw.clamp(1.0, duration_sec.max(1.0))
}

fn kill_pid(pid: u32) -> Result<(), String> {
    #[cfg(windows)]
    {
        let mut tk = Command::new("taskkill");
        tk.args(["/PID", &pid.to_string(), "/F", "/T"]);
        suppress_child_console(&mut tk);
        let status = tk.status().map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("Não foi possível encerrar o processo do FFmpeg.".into());
        }
    }
    #[cfg(not(windows))]
    {
        Command::new("kill")
            .args(["-9", &pid.to_string()])
            .status()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn parse_ffmpeg_time_ratio(line: &str, duration_sec: f64) -> Option<f64> {
    if duration_sec <= 0.0 {
        return None;
    }
    let re = Regex::new(r"time=(\d+):(\d+):(\d+\.\d+)").ok()?;
    let cap = re.captures(line)?;
    let h: f64 = cap.get(1)?.as_str().parse().ok()?;
    let m: f64 = cap.get(2)?.as_str().parse().ok()?;
    let s: f64 = cap.get(3)?.as_str().parse().ok()?;
    let t = h * 3600.0 + m * 60.0 + s;
    Some((t / duration_sec).clamp(0.0, 1.0))
}

fn subtitle_stem_matches_video(video: &Path, srt: &Path) -> bool {
    let v = video.file_stem().and_then(|s| s.to_str());
    let s = srt.file_stem().and_then(|x| x.to_str());
    match (v, s) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        _ => false,
    }
}

fn collect_segment_video_parts(
    dir: &Path,
    stem: &str,
    ext: &str,
    not_before: SystemTime,
) -> Result<Vec<PathBuf>, String> {
    let prefix = format!("{stem}_part_");
    let suffix = format!(".{ext}");
    let margin = std::time::Duration::from_secs(5);
    let threshold = not_before
        .checked_sub(margin)
        .unwrap_or(not_before);

    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| format!("Falha ao ler pasta de saída: {e}"))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let name_ok = p
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&prefix) && n.ends_with(&suffix));
            if !name_ok {
                return false;
            }
            let Ok(meta) = fs::metadata(p) else {
                return false;
            };
            let Ok(m) = meta.modified() else {
                return false;
            };
            m >= threshold
        })
        .collect();
    paths.sort();
    if paths.is_empty() {
        return Err(
            "Nenhuma parte de vídeo encontrada após o FFmpeg (ou arquivos antigos na pasta?). \n\
             Dica: use pasta vazia ou apague partes anteriores com o mesmo nome."
                .into(),
        );
    }
    Ok(paths)
}

fn split_external_srt_for_parts(
    srt_path: &Path,
    output_dir: &Path,
    stem: &str,
    video_ext: &str,
    app_handle: &AppHandle,
    parts_not_before: SystemTime,
) -> Result<(), String> {
    let cues = srt::parse_srt_file(srt_path)?;
    let video_parts = collect_segment_video_parts(output_dir, stem, video_ext, parts_not_before)?;

    let _ = app_handle.emit(
        "split-progress",
        &SplitProgressPayload {
            line: format!(
                "Ajustando legendas ({}) partes com base na duração real de cada arquivo…",
                video_parts.len()
            ),
            ratio: None,
        },
    );

    let mut durations_ms = Vec::with_capacity(video_parts.len());
    for vp in &video_parts {
        let d = probe_duration_only(&vp.to_string_lossy())?;
        durations_ms.push((d * 1000.0).round() as i64);
    }

    let out_srts: Vec<PathBuf> = video_parts
        .iter()
        .map(|p| p.with_extension("srt"))
        .collect();

    srt::write_split_srts(&cues, &durations_ms, &out_srts)?;

    let _ = app_handle.emit(
        "split-progress",
        &SplitProgressPayload {
            line: format!(
                "SRT: {} arquivo(s) gerado(s) (tempos relativos a cada parte).",
                out_srts.len()
            ),
            ratio: None,
        },
    );

    Ok(())
}

#[tauri::command]
fn split_cancel(state: State<'_, Arc<SplitState>>) -> Result<(), String> {
    let mut g = state
        .current_pid
        .lock()
        .map_err(|_| "Estado interno bloqueado.".to_string())?;
    if let Some(pid) = g.take() {
        kill_pid(pid)?;
    }
    Ok(())
}

#[tauri::command]
fn split_video_start(
    app: AppHandle,
    state: State<'_, Arc<SplitState>>,
    input_path: String,
    output_dir: String,
    max_part_gb: f64,
    subtitle_path: Option<String>,
) -> Result<(), String> {
    if max_part_gb <= 0.0 {
        return Err("O tamanho máximo por parte deve ser maior que zero.".into());
    }
    resolve_bin("ffmpeg").ok_or("ffmpeg não encontrado no PATH.")?;
    let probe = probe_file(input_path.clone())?;
    let max_part_bytes = (max_part_gb * 1024.0_f64 * 1024.0 * 1024.0) as u64;
    let segment_seconds =
        estimate_segment_seconds(max_part_bytes, probe.size_bytes, probe.duration_sec);
    let seg_fmt = segment_format_for_ext(&probe.extension);

    let out_dir = Path::new(&output_dir);
    if !out_dir.is_dir() {
        return Err("Pasta de saída não existe ou não é uma pasta.".into());
    }

    let input_path_buf = PathBuf::from(&input_path);
    let stem = input_path_buf
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("video")
        .to_string();
    let ext = probe.extension.clone();
    let out_pattern = out_dir.join(format!("{}_part_%03d.{}", stem, ext));

    if let Some(ref sp) = subtitle_path {
        let p = Path::new(sp);
        if !p.is_file() {
            return Err(format!("Arquivo de legendas não encontrado: {}", p.display()));
        }
        if !p
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("srt"))
        {
            return Err("O arquivo de legendas deve ter extensão .srt.".into());
        }
        if !subtitle_stem_matches_video(&input_path_buf, p) {
            let vid = input_path_buf
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("?");
            let leg = p.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
            return Err(format!(
                "O nome do arquivo de legendas (sem .srt) deve ser o mesmo do vídeo (sem extensão). \
                 Vídeo: «{vid}», legendas: «{leg}»."
            ));
        }
    }

    {
        let mut g = state
            .current_pid
            .lock()
            .map_err(|_| "Estado interno bloqueado.".to_string())?;
        if let Some(pid) = *g {
            let _ = kill_pid(pid);
        }
        *g = None;
    }

    let duration_for_progress = probe.duration_sec;
    let app_handle = app.clone();
    let state_arc = Arc::clone(&*state);

    let ffmpeg_path = resolve_bin("ffmpeg").ok_or("ffmpeg não encontrado.")?;
    let out_pat_str = out_pattern.to_string_lossy().into_owned();
    let seg_time = format!("{segment_seconds:.3}");
    let subtitle_path_owned = subtitle_path.clone();
    let stem_owned = stem.clone();
    let ext_owned = ext.clone();
    let output_dir_owned = out_dir.to_path_buf();

    std::thread::spawn(move || {
        let run = || -> Result<(), String> {
            let job_start = SystemTime::now();
            let mut cmd = Command::new(&ffmpeg_path);
            cmd.arg("-hide_banner")
                .arg("-nostdin")
                .arg("-i")
                .arg(&input_path)
                .arg("-map")
                .arg("0")
                .arg("-c")
                .arg("copy")
                .arg("-f")
                .arg("segment")
                .arg("-segment_time")
                .arg(&seg_time)
                .arg("-segment_format")
                .arg(seg_fmt)
                .arg("-reset_timestamps")
                .arg("1")
                .arg(&out_pat_str)
                .stderr(Stdio::piped());
            suppress_child_console(&mut cmd);

            let mut child = cmd
                .spawn()
                .map_err(|e| format!("Falha ao iniciar o FFmpeg: {e}"))?;
            let pid = child.id();
            {
                if let Ok(mut g) = state_arc.current_pid.lock() {
                    *g = Some(pid);
                }
            }

            let stderr = child.stderr.take().ok_or("stderr do FFmpeg indisponível.")?;
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                let ratio = parse_ffmpeg_time_ratio(&line, duration_for_progress);
                let payload = SplitProgressPayload { line, ratio };
                let _ = app_handle.emit("split-progress", &payload);
            }

            let status = child.wait().map_err(|e| e.to_string())?;
            {
                if let Ok(mut g) = state_arc.current_pid.lock() {
                    *g = None;
                }
            }
            if !status.success() {
                return Err(
                    "O FFmpeg terminou com erro. Verifique o log ou tente contêiner MKV.".into(),
                );
            }

            if let Some(ref sp) = subtitle_path_owned {
                split_external_srt_for_parts(
                    Path::new(sp),
                    &output_dir_owned,
                    &stem_owned,
                    &ext_owned,
                    &app_handle,
                    job_start,
                )?;
            }

            Ok(())
        };

        match run() {
            Ok(_) => {
                let _ = app_handle.emit("split-done", &serde_json::json!({ "ok": true }));
            }
            Err(e) => {
                let _ = app_handle.emit("split-error", &serde_json::json!({ "message": e }));
            }
        }
    });

    Ok(())
}

fn is_mp4_like(path: &Path) -> bool {
    path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("mp4") || e.eq_ignore_ascii_case("m4v"))
}

#[tauri::command]
fn convert_mp4_to_mkv_start(
    app: AppHandle,
    state: State<'_, Arc<SplitState>>,
    input_path: String,
    output_dir: String,
) -> Result<(), String> {
    resolve_bin("ffmpeg").ok_or("ffmpeg não encontrado no PATH.")?;

    let input_path_buf = PathBuf::from(&input_path);
    if !input_path_buf.is_file() {
        return Err("Arquivo de entrada não encontrado.".into());
    }
    if !is_mp4_like(&input_path_buf) {
        return Err(
            "A conversão MP4 → MKV aceita apenas arquivos .mp4 ou .m4v.".into(),
        );
    }

    let out_dir = Path::new(&output_dir);
    if !out_dir.is_dir() {
        return Err("Pasta de saída não existe ou não é uma pasta.".into());
    }

    let stem = input_path_buf
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("video")
        .to_string();
    let output_path = out_dir.join(format!("{stem}.mkv"));
    if output_path.exists() {
        return Err(format!(
            "Já existe um arquivo na saída: {}. Apague ou renomeie antes de converter.",
            output_path.display()
        ));
    }

    let duration_for_progress = probe_duration_only(&input_path)?;

    {
        let mut g = state
            .current_pid
            .lock()
            .map_err(|_| "Estado interno bloqueado.".to_string())?;
        if let Some(pid) = *g {
            let _ = kill_pid(pid);
        }
        *g = None;
    }

    let app_handle = app.clone();
    let state_arc = Arc::clone(&*state);
    let ffmpeg_path = resolve_bin("ffmpeg").ok_or("ffmpeg não encontrado.")?;
    let out_str = output_path.to_string_lossy().into_owned();

    std::thread::spawn(move || {
        let run = || -> Result<(), String> {
            let mut cmd = Command::new(&ffmpeg_path);
            cmd.arg("-hide_banner")
                .arg("-nostdin")
                .arg("-i")
                .arg(&input_path)
                .arg("-map")
                .arg("0")
                .arg("-c")
                .arg("copy")
                .arg(&out_str)
                .stderr(Stdio::piped());
            suppress_child_console(&mut cmd);

            let mut child = cmd
                .spawn()
                .map_err(|e| format!("Falha ao iniciar o FFmpeg: {e}"))?;
            let pid = child.id();
            {
                if let Ok(mut g) = state_arc.current_pid.lock() {
                    *g = Some(pid);
                }
            }

            let stderr = child.stderr.take().ok_or("stderr do FFmpeg indisponível.")?;
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                let ratio = parse_ffmpeg_time_ratio(&line, duration_for_progress);
                let payload = SplitProgressPayload { line, ratio };
                let _ = app_handle.emit("convert-progress", &payload);
            }

            let status = child.wait().map_err(|e| e.to_string())?;
            {
                if let Ok(mut g) = state_arc.current_pid.lock() {
                    *g = None;
                }
            }
            if !status.success() {
                let _ = fs::remove_file(&out_str);
                return Err(
                    "O FFmpeg terminou com erro (remux MP4→MKV). Fluxos ou legendas podem ser incompatíveis com MKV.".into(),
                );
            }

            Ok(())
        };

        match run() {
            Ok(_) => {
                let _ = app_handle.emit("convert-done", &serde_json::json!({ "ok": true }));
            }
            Err(e) => {
                let _ = app_handle.emit("convert-error", &serde_json::json!({ "message": e }));
            }
        }
    });

    Ok(())
}

#[cfg(test)]
mod stem_tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn subtitle_stem_matches_ignore_case() {
        assert!(subtitle_stem_matches_video(
            Path::new(r"C:\a\Filme.mkv"),
            Path::new(r"D:\x\filme.srt")
        ));
        assert!(!subtitle_stem_matches_video(
            Path::new("a.mkv"),
            Path::new("b.srt")
        ));
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Arc::new(SplitState {
            current_pid: Mutex::new(None),
        }))
        .invoke_handler(tauri::generate_handler![
            ffmpeg_status,
            probe_file,
            split_video_start,
            split_cancel,
            convert_mp4_to_mkv_start,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
