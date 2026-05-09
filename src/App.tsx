import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useRef, useState } from "react";
import "./App.css";

type FfmpegStatus = {
  available: boolean;
  ffmpegPath: string | null;
  ffprobePath: string | null;
  message: string;
};

type SplitProgressPayload = {
  line: string;
  ratio: number | null;
};

type SplitErrorPayload = { message: string };

type AppMode = "split" | "convert";

function parseGb(input: string): number | null {
  const n = Number(input.trim().replace(",", "."));
  if (!Number.isFinite(n) || n <= 0) return null;
  return n;
}

export default function App() {
  const [mode, setMode] = useState<AppMode>("split");
  const [ffmpegInfo, setFfmpegInfo] = useState<FfmpegStatus | null>(null);
  const [filePaths, setFilePaths] = useState<string[]>([]);
  const [outputDir, setOutputDir] = useState<string>("");
  const [subtitlePath, setSubtitlePath] = useState<string | null>(null);
  const [gbInput, setGbInput] = useState("2");
  const [convertPaths, setConvertPaths] = useState<string[]>([]);
  const [convertOutputDir, setConvertOutputDir] = useState("");
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState(0);
  const [logLines, setLogLines] = useState<string[]>([]);
  const logEndRef = useRef<HTMLDivElement | null>(null);

  const queueRef = useRef<string[]>([]);
  const outputDirRef = useRef("");
  const maxGbRef = useRef(1);
  const queueIndexRef = useRef(0);
  const subtitlePathRef = useRef<string | null>(null);
  const runQueueRef = useRef<(() => void) | null>(null);

  const convertQueueRef = useRef<string[]>([]);
  const convertOutDirRef = useRef("");
  const convertIndexRef = useRef(0);
  const runConvertRef = useRef<(() => void) | null>(null);

  const appendLog = useCallback((line: string) => {
    setLogLines((prev) => [...prev.slice(-400), line]);
  }, []);

  useEffect(() => {
    void invoke<FfmpegStatus>("ffmpeg_status").then(setFfmpegInfo);
  }, []);

  useEffect(() => {
    logEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [logLines]);

  const runNextInQueue = useCallback(() => {
    const files = queueRef.current;
    const i = queueIndexRef.current;
    if (i >= files.length) {
      setBusy(false);
      setProgress(1);
      appendLog("Fila concluída.");
      return;
    }
    const path = files[i];
    appendLog(`— Partindo (${i + 1}/${files.length}) —`);
    appendLog(path);
    const out = outputDirRef.current;
    const gb = maxGbRef.current;
    const sub = subtitlePathRef.current;
    void invoke("split_video_start", {
      inputPath: path,
      outputDir: out,
      maxPartGb: gb,
      subtitlePath: sub,
    }).catch((e: unknown) => {
      const msg = e instanceof Error ? e.message : String(e);
      appendLog(`Erro: ${msg}`);
      setBusy(false);
      setProgress(0);
    });
  }, [appendLog]);

  const runNextConvert = useCallback(() => {
    const files = convertQueueRef.current;
    const i = convertIndexRef.current;
    if (i >= files.length) {
      setBusy(false);
      setProgress(1);
      appendLog("Conversões concluídas.");
      return;
    }
    const path = files[i];
    appendLog(`— Converter (${i + 1}/${files.length}) —`);
    appendLog(path);
    void invoke("convert_mp4_to_mkv_start", {
      inputPath: path,
      outputDir: convertOutDirRef.current,
    }).catch((e: unknown) => {
      const msg = e instanceof Error ? e.message : String(e);
      appendLog(`Erro: ${msg}`);
      setBusy(false);
      setProgress(0);
    });
  }, [appendLog]);

  useEffect(() => {
    runQueueRef.current = runNextInQueue;
  }, [runNextInQueue]);

  useEffect(() => {
    runConvertRef.current = runNextConvert;
  }, [runNextConvert]);

  useEffect(() => {
    let unProgress: UnlistenFn | undefined;
    let unDone: UnlistenFn | undefined;
    let unErr: UnlistenFn | undefined;

    void listen<SplitProgressPayload>("split-progress", (ev) => {
      if (ev.payload.ratio != null) {
        const n = queueRef.current.length;
        if (n > 0) {
          const i = queueIndexRef.current;
          const overall = Math.min(1, (i + ev.payload.ratio) / n);
          setProgress(overall);
        }
      }
      if (ev.payload.line) appendLog(ev.payload.line);
    }).then((fn) => {
      unProgress = fn;
    });

    void listen("split-done", () => {
      appendLog("Trecho finalizado.");
      queueIndexRef.current += 1;
      runQueueRef.current?.();
    }).then((fn) => {
      unDone = fn;
    });

    void listen<SplitErrorPayload>("split-error", (ev) => {
      appendLog(`Erro FFmpeg: ${ev.payload.message}`);
      setBusy(false);
      setProgress(0);
    }).then((fn) => {
      unErr = fn;
    });

    return () => {
      void unProgress?.();
      void unDone?.();
      void unErr?.();
    };
  }, [appendLog]);

  useEffect(() => {
    let unProgress: UnlistenFn | undefined;
    let unDone: UnlistenFn | undefined;
    let unErr: UnlistenFn | undefined;

    void listen<SplitProgressPayload>("convert-progress", (ev) => {
      if (ev.payload.ratio != null) {
        const n = convertQueueRef.current.length;
        if (n > 0) {
          const i = convertIndexRef.current;
          const overall = Math.min(1, (i + ev.payload.ratio) / n);
          setProgress(overall);
        }
      }
      if (ev.payload.line) appendLog(ev.payload.line);
    }).then((fn) => {
      unProgress = fn;
    });

    void listen("convert-done", () => {
      appendLog("Conversão finalizada.");
      convertIndexRef.current += 1;
      runConvertRef.current?.();
    }).then((fn) => {
      unDone = fn;
    });

    void listen<SplitErrorPayload>("convert-error", (ev) => {
      appendLog(`Erro FFmpeg: ${ev.payload.message}`);
      setBusy(false);
      setProgress(0);
    }).then((fn) => {
      unErr = fn;
    });

    return () => {
      void unProgress?.();
      void unDone?.();
      void unErr?.();
    };
  }, [appendLog]);

  const pickVideos = async () => {
    const sel = await open({
      multiple: true,
      filters: [
        {
          name: "Vídeo",
          extensions: ["mkv", "mp4", "mov", "webm", "avi", "ts", "m2ts", "m4v"],
        },
      ],
      title: "Selecionar vídeos",
    });
    if (sel == null) return;
    const list = Array.isArray(sel) ? sel : [sel];
    setFilePaths(list);
    if (!outputDir && list.length > 0) {
      const first = list[0].replace(/[/\\][^/\\]+$/, "");
      setOutputDir(first);
      outputDirRef.current = first;
    }
  };

  const pickMp4ForConvert = async () => {
    const sel = await open({
      multiple: true,
      filters: [{ name: "MP4 / M4V", extensions: ["mp4", "m4v"] }],
      title: "Selecionar MP4 para converter a MKV",
    });
    if (sel == null) return;
    const list = Array.isArray(sel) ? sel : [sel];
    setConvertPaths(list);
    if (!convertOutputDir && list.length > 0) {
      const first = list[0].replace(/[/\\][^/\\]+$/, "");
      setConvertOutputDir(first);
      convertOutDirRef.current = first;
    }
  };

  const pickFolder = async () => {
    const sel = await open({
      directory: true,
      title: "Pasta de saída",
    });
    if (typeof sel === "string") {
      setOutputDir(sel);
      outputDirRef.current = sel;
    }
  };

  const pickConvertFolder = async () => {
    const sel = await open({
      directory: true,
      title: "Pasta de saída (MKV)",
    });
    if (typeof sel === "string") {
      setConvertOutputDir(sel);
      convertOutDirRef.current = sel;
    }
  };

  const startQueue = () => {
    const gb = parseGb(gbInput);
    if (gb == null) {
      appendLog("Informe um tamanho válido em GB (ex.: 1,5 ou 2).");
      return;
    }
    if (!ffmpegInfo?.available) {
      appendLog("FFmpeg não está disponível. Instale e adicione ao PATH.");
      return;
    }
    if (filePaths.length === 0) {
      appendLog("Selecione ao menos um arquivo de vídeo.");
      return;
    }
    if (!outputDir) {
      appendLog("Escolha a pasta de saída.");
      return;
    }

    maxGbRef.current = gb;
    outputDirRef.current = outputDir;
    subtitlePathRef.current = subtitlePath;
    queueRef.current = [...filePaths];
    queueIndexRef.current = 0;
    setBusy(true);
    setProgress(0);
    setLogLines([]);
    appendLog(`Teto aproximado por parte: ${gb} GB (cópia de fluxo; cortes em keyframes).`);
    if (subtitlePath) {
      appendLog(`SRT: ${subtitlePath}`);
    }
    runNextInQueue();
  };

  const startConvertQueue = () => {
    if (!ffmpegInfo?.available) {
      appendLog("FFmpeg não está disponível. Instale e adicione ao PATH.");
      return;
    }
    if (convertPaths.length === 0) {
      appendLog("Selecione ao menos um arquivo .mp4 ou .m4v.");
      return;
    }
    if (!convertOutputDir) {
      appendLog("Escolha a pasta de saída.");
      return;
    }

    convertOutDirRef.current = convertOutputDir;
    convertQueueRef.current = [...convertPaths];
    convertIndexRef.current = 0;
    setBusy(true);
    setProgress(0);
    setLogLines([]);
    appendLog(
      "MP4/M4V → MKV para TV: re-encoding H.264 High nível 4.1, 30 fps, AAC estéreo 48 kHz; legendas copiadas quando o FFmpeg permitir (demora mais que remux)."
    );
    runNextConvert();
  };

  const pickSrt = async () => {
    const sel = await open({
      multiple: false,
      filters: [{ name: "SubRip", extensions: ["srt"] }],
      title: "Arquivo de legendas (.srt)",
    });
    if (typeof sel === "string") {
      setSubtitlePath(sel);
    }
  };

  const clearSrt = () => setSubtitlePath(null);

  const cancel = () => {
    void invoke("split_cancel").catch(() => {});
    appendLog("Cancelamento solicitado…");
    setBusy(false);
    setProgress(0);
  };

  return (
    <div className="app">
      <header className="header">
        <h1>VideoDivider</h1>
        <p className="subtitle">
          Divide MKV, MP4 e outros vídeos em partes próximas ao tamanho em GB (via FFmpeg, sem
          re-encoding), ou converta MP4/M4V para MKV com perfil compatível com TVs.
        </p>
      </header>

      <div className="mode-tabs">
        <button
          type="button"
          className={mode === "split" ? "active" : ""}
          onClick={() => setMode("split")}
          disabled={busy}
        >
          Dividir por tamanho
        </button>
        <button
          type="button"
          className={mode === "convert" ? "active" : ""}
          onClick={() => setMode("convert")}
          disabled={busy}
        >
          MP4 → MKV (TV)
        </button>
      </div>

      <section className="panel">
        <div className="status-pill" data-ok={ffmpegInfo?.available ?? false}>
          {ffmpegInfo?.available ? "FFmpeg OK" : "FFmpeg ausente"}
        </div>
        {ffmpegInfo && <p className="hint">{ffmpegInfo.message}</p>}
      </section>

      {mode === "split" ? (
        <section className="panel grid">
          <label className="field">
            <span>Arquivos</span>
            <div className="field-row">
              <button type="button" onClick={pickVideos} disabled={busy}>
                Escolher vídeos…
              </button>
              <span className="muted">
                {filePaths.length === 0 ? "Nenhum" : `${filePaths.length} selecionado(s)`}
              </span>
            </div>
          </label>

          <label className="field">
            <span>Pasta de saída</span>
            <div className="field-row">
              <button type="button" onClick={pickFolder} disabled={busy}>
                Escolher pasta…
              </button>
              <span className="path-truncate muted" title={outputDir}>
                {outputDir || "—"}
              </span>
            </div>
          </label>

          <label className="field">
            <span>Legendas externas (.srt), opcional</span>
            <div className="field-row">
              <button type="button" onClick={pickSrt} disabled={busy}>
                Escolher .srt…
              </button>
              <button type="button" onClick={clearSrt} disabled={busy || !subtitlePath}>
                Limpar
              </button>
              <span className="path-truncate muted" title={subtitlePath ?? ""}>
                {subtitlePath ? subtitlePath.replace(/^.*[/\\]/, "") : "Nenhum"}
              </span>
            </div>
          </label>
          <label className="field">
            <span>Máx. por parte (GB)</span>
            <input
              type="text"
              inputMode="decimal"
              value={gbInput}
              onChange={(e) => setGbInput(e.target.value)}
              disabled={busy}
              placeholder="ex.: 1,5"
            />
          </label>
        </section>
      ) : (
        <section className="panel grid">
          <label className="field">
            <span>Arquivos MP4 ou M4V</span>
            <div className="field-row">
              <button type="button" onClick={pickMp4ForConvert} disabled={busy}>
                Escolher .mp4…
              </button>
              <span className="muted">
                {convertPaths.length === 0 ? "Nenhum" : `${convertPaths.length} selecionado(s)`}
              </span>
            </div>
          </label>

          <label className="field">
            <span>Pasta de saída</span>
            <div className="field-row">
              <button type="button" onClick={pickConvertFolder} disabled={busy}>
                Escolher pasta…
              </button>
              <span className="path-truncate muted" title={convertOutputDir}>
                {convertOutputDir || "—"}
              </span>
            </div>
          </label>

          <p className="hint" style={{ margin: 0 }}>
            Gera <code>nome.mkv</code> na pasta escolhida (mesmo nome base do ficheiro). Ajusta vídeo e
            áudio para leitura em muitas TVs (ex.: evita 1080p60/nível H.264 que alguns aparelhos não
            suportam). Se já existir um <code>.mkv</code> com esse nome, não sobrescreve.
          </p>
        </section>
      )}

      <div className="actions">
        {mode === "split" ? (
          <button type="button" className="primary" onClick={startQueue} disabled={busy}>
            Iniciar divisão
          </button>
        ) : (
          <button type="button" className="primary" onClick={startConvertQueue} disabled={busy}>
            Iniciar conversão
          </button>
        )}
        <button type="button" onClick={cancel} disabled={!busy}>
          Cancelar FFmpeg
        </button>
      </div>

      <div className="progress-wrap">
        <div className="progress-bar" role="progressbar" aria-valuenow={Math.round(progress * 100)}>
          <div className="progress-fill" style={{ width: `${Math.min(100, progress * 100)}%` }} />
        </div>
        <span className="muted small">{Math.round(progress * 100)}%</span>
      </div>

      <section className="log-panel">
        <div className="log-header">Log</div>
        <pre className="log">
          {logLines.join("\n")}
          <div ref={logEndRef} />
        </pre>
      </section>

      <footer className="footer muted small">
        {mode === "split" ? (
          <>
            Com <code>-c copy</code>, o tamanho real de cada parte pode variar em torno do valor
            alvo. Com .srt externo, o nome do ficheiro (sem extensão) tem de coincidir com o do
            vídeo (ex.: <code>Filme.mkv</code> e <code>Filme.srt</code>). Comparação ignora
            maiúsculas/minúsculas. Saída: <code>nome_part_XXX.srt</code> por cada parte. Vários vídeos
            na fila exigem o mesmo par nome/SRT em cada item.
          </>
        ) : (
          <>
            Modo MP4 → MKV (TV): re-encoding para H.264 High nível 4.1 a 30 fps, AAC estéreo 48 kHz.
            Demora mais que um remux e há pequena perda de qualidade típica de CRF 20. Legendas são
            copiadas quando o FFmpeg as aceita no MKV.
          </>
        )}
      </footer>
    </div>
  );
}
