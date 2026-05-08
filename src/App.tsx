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

function parseGb(input: string): number | null {
  const n = Number(input.trim().replace(",", "."));
  if (!Number.isFinite(n) || n <= 0) return null;
  return n;
}

export default function App() {
  const [ffmpegInfo, setFfmpegInfo] = useState<FfmpegStatus | null>(null);
  const [filePaths, setFilePaths] = useState<string[]>([]);
  const [outputDir, setOutputDir] = useState<string>("");
  const [subtitlePath, setSubtitlePath] = useState<string | null>(null);
  const [gbInput, setGbInput] = useState("2");
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

  useEffect(() => {
    runQueueRef.current = runNextInQueue;
  }, [runNextInQueue]);

  useEffect(() => {
    let unProgress: UnlistenFn | undefined;
    let unDone: UnlistenFn | undefined;
    let unErr: UnlistenFn | undefined;

    void listen<SplitProgressPayload>("split-progress", (ev) => {
      if (ev.payload.ratio != null) setProgress(ev.payload.ratio);
      if (ev.payload.line) appendLog(ev.payload.line);
    }).then((fn) => {
      unProgress = fn;
    });

    void listen("split-done", () => {
      appendLog("Trecho finalizado.");
      queueIndexRef.current += 1;
      setProgress(0);
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
          re-encoding).
        </p>
      </header>

      <section className="panel">
        <div className="status-pill" data-ok={ffmpegInfo?.available ?? false}>
          {ffmpegInfo?.available ? "FFmpeg OK" : "FFmpeg ausente"}
        </div>
        {ffmpegInfo && (
          <p className="hint">{ffmpegInfo.message}</p>
        )}
      </section>

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

      <div className="actions">
        <button type="button" className="primary" onClick={startQueue} disabled={busy}>
          Iniciar
        </button>
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
        Com <code>-c copy</code>, o tamanho real de cada parte pode variar em torno do valor alvo.
        Com .srt externo, o nome do ficheiro (sem extensão) tem de coincidir com o do vídeo (ex.:{" "}
        <code>Filme.mkv</code> e <code>Filme.srt</code>). Comparação ignora maiúsculas/minúsculas.
        Saída: <code>nome_part_XXX.srt</code> por cada parte. Vários vídeos na fila exigem o mesmo
        par nome/SRT em cada item.
      </footer>
    </div>
  );
}
