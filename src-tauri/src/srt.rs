use encoding_rs::{UTF_16BE, UTF_16LE, WINDOWS_1252};
use regex::Regex;
use std::fs;
use std::path::Path;

/// Marca de ordem de byte UTF-8 nos ficheiros gerados (melhora deteção em TVs / leitores no Windows).
const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

#[derive(Debug, Clone)]
pub struct SrtCue {
    #[allow(dead_code)]
    pub index: usize,
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

fn parse_ts(h: &str, m: &str, s: &str, ms: &str) -> Option<i64> {
    let hh: i64 = h.parse().ok()?;
    let mm: i64 = m.parse().ok()?;
    let ss: i64 = s.parse().ok()?;
    let mss: i64 = ms.parse().ok()?;
    Some(hh * 3_600_000 + mm * 60_000 + ss * 1_000 + mss)
}

/// Decodifica bytes de .srt para texto Unicode: UTF-8 (com ou sem BOM), UTF-16 LE/BE com BOM,
/// ou — se não for UTF-8 válido — Windows-1252 (comum em legendas PT/BR antigas).
fn decode_srt_bytes(bytes: &[u8]) -> Result<String, String> {
    if bytes.starts_with(b"\xEF\xBB\xBF") {
        let payload = &bytes[3..];
        return std::str::from_utf8(payload)
            .map(|s| s.to_string())
            .map_err(|_| "Ficheiro com BOM UTF-8 mas corpo não é UTF-8 válido.".into());
    }

    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let (cow, _, _) = UTF_16LE.decode(bytes);
        return Ok(cow.into_owned());
    }

    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let (cow, _, _) = UTF_16BE.decode(bytes);
        return Ok(cow.into_owned());
    }

    if let Ok(s) = std::str::from_utf8(bytes) {
        return Ok(s.to_string());
    }

    let (cow, _, _) = WINDOWS_1252.decode(bytes);
    Ok(cow.into_owned())
}

/// Lê e interpreta um arquivo .srt (UTF-8, UTF-16 com BOM, ou Windows-1252).
pub fn parse_srt_file(path: &Path) -> Result<Vec<SrtCue>, String> {
    let bytes = fs::read(path).map_err(|e| format!("Erro ao ler SRT: {e}"))?;
    let raw = decode_srt_bytes(&bytes)?;
    parse_srt(&raw)
}

fn parse_srt(raw: &str) -> Result<Vec<SrtCue>, String> {
    let re_time = Regex::new(
        r"(?m)^(\d{2}):(\d{2}):(\d{2})[,.](\d{3})\s*-->\s*(\d{2}):(\d{2}):(\d{2})[,.](\d{3})",
    )
    .map_err(|e| e.to_string())?;

    let text = raw
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");

    let mut cues = Vec::new();

    for block in text.split("\n\n").map(str::trim).filter(|b| !b.is_empty()) {
        let lines: Vec<&str> = block.lines().collect();
        if lines.len() < 2 {
            continue;
        }
        let time_line = lines[1];
        let Some(cap) = re_time.captures(time_line) else {
            continue;
        };
        let start_ms = parse_ts(&cap[1], &cap[2], &cap[3], &cap[4])
            .ok_or_else(|| "Timestamp inicial inválido.".to_string())?;
        let end_ms = parse_ts(&cap[5], &cap[6], &cap[7], &cap[8])
            .ok_or_else(|| "Timestamp final inválido.".to_string())?;
        if end_ms <= start_ms {
            continue;
        }
        let index: usize = lines[0].trim().parse().unwrap_or(0);
        let text_body = lines[2..].join("\n");
        cues.push(SrtCue {
            index,
            start_ms,
            end_ms,
            text: text_body,
        });
    }

    if cues.is_empty() {
        return Err("Nenhuma legenda válida encontrada no SRT.".into());
    }

    Ok(cues)
}

fn format_ts(ms: i64) -> String {
    let ms = ms.max(0);
    let h = ms / 3_600_000;
    let m = (ms % 3_600_000) / 60_000;
    let s = (ms % 60_000) / 1_000;
    let x = ms % 1_000;
    format!("{h:02}:{m:02}:{s:02},{x:03}")
}

/// Gera blocos SRT com tempos relativos à parte (0 = início do pedaço de vídeo).
fn cues_for_part(
    cues: &[SrtCue],
    part_offset_ms: i64,
    part_duration_ms: i64,
) -> Vec<SrtCue> {
    let win_start = part_offset_ms;
    let win_end = part_offset_ms + part_duration_ms;
    let mut out = Vec::new();

    for c in cues {
        if c.end_ms <= win_start || c.start_ms >= win_end {
            continue;
        }
        let new_start = (c.start_ms - win_start).max(0);
        let new_end = (c.end_ms - win_start).min(part_duration_ms);
        if new_end <= new_start {
            continue;
        }
        out.push(SrtCue {
            index: 0,
            start_ms: new_start,
            end_ms: new_end,
            text: c.text.clone(),
        });
    }

    out
}

fn serialize_srt(cues: &[SrtCue]) -> String {
    let mut buf = String::new();
    for (i, c) in cues.iter().enumerate() {
        buf.push_str(&format!("{}\n", i + 1));
        buf.push_str(&format!(
            "{} --> {}\n",
            format_ts(c.start_ms),
            format_ts(c.end_ms)
        ));
        buf.push_str(&c.text);
        buf.push_str("\n\n");
    }
    buf
}

/// Corta legendas para cada parte, usando janelas [acumulado, acumulado + duração_da_parte).
pub fn write_split_srts(
    cues: &[SrtCue],
    part_durations_ms: &[i64],
    output_paths: &[std::path::PathBuf],
) -> Result<(), String> {
    if part_durations_ms.len() != output_paths.len() {
        return Err("Número de partes e caminhos SRT não coincidem.".into());
    }

    let mut offset_ms = 0_i64;
    for (i, dur) in part_durations_ms.iter().enumerate() {
        let part_cues = cues_for_part(cues, offset_ms, *dur);
        let body = serialize_srt(&part_cues);
        let mut out = Vec::with_capacity(UTF8_BOM.len() + body.len());
        out.extend_from_slice(UTF8_BOM);
        out.extend_from_slice(body.as_bytes());
        fs::write(&output_paths[i], out).map_err(|e| format!("Erro ao gravar SRT: {e}"))?;
        offset_ms += *dur;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_utf16_le_with_bom() {
        let text = "1\n00:00:01,000 --> 00:00:02,000\nOlá\n\n";
        let mut bytes = vec![0xFFu8, 0xFEu8];
        for u in text.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        let s = decode_srt_bytes(&bytes).expect("decode");
        let cues = parse_srt(&s).unwrap();
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text, "Olá");
    }

    #[test]
    fn split_srt_output_starts_with_utf8_bom() {
        let dir = std::env::temp_dir().join(format!(
            "videodivider-srt-bom-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");
        let out = dir.join("part.srt");
        let cues = vec![SrtCue {
            index: 1,
            start_ms: 0,
            end_ms: 2_000,
            text: "teste".into(),
        }];
        write_split_srts(&cues, &[5_000], &[out.clone()]).expect("write");
        let written = fs::read(&out).expect("read");
        assert!(
            written.starts_with(UTF8_BOM),
            "SRT gerado deve começar com BOM UTF-8"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_cp1252_invalid_utf8() {
        // Bytes comuns em SRT antigo (Windows-1252): ç = 0xE7, ã = 0xE3 — não são UTF-8 válidos juntos assim.
        let bytes = b"1\n00:00:01,000 --> 00:00:02,000\nTeste \xE7\xE3o\n\n";
        let s = decode_srt_bytes(bytes).expect("decode");
        let cues = parse_srt(&s).unwrap();
        assert_eq!(cues.len(), 1);
        assert!(cues[0].text.contains("ç") || cues[0].text.contains("ã"));
    }

    #[test]
    fn parses_sample_block() {
        let raw = "1\n00:01:43,053 --> 00:01:46,265\nOlá\n\n2\n00:01:46,348 --> 00:01:48,142\nOi";
        let cues = parse_srt(raw).unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].start_ms, 103_053);
        assert_eq!(cues[0].end_ms, 106_265);
    }

    #[test]
    fn clip_cue_across_boundary() {
        let cues = vec![SrtCue {
            index: 1,
            start_ms: 90_000,
            end_ms: 120_000,
            text: "x".into(),
        }];
        let part = cues_for_part(&cues, 60_000, 40_000);
        assert_eq!(part.len(), 1);
        assert_eq!(part[0].start_ms, 30_000);
        assert_eq!(part[0].end_ms, 40_000);
    }
}
