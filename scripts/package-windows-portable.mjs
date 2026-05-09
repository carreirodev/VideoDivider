/**
 * Copia o .exe de release para dist-portable/ e gera um .zip (Windows).
 * Pré-requisito: `npm run tauri build` (ou npm run tauri:win-portable).
 */
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  unlinkSync,
} from "node:fs";
import { execFileSync } from "node:child_process";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = join(__dirname, "..");
const tauriConf = JSON.parse(
  readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8"),
);

const version = tauriConf.version;
const productSafe = tauriConf.productName.replace(/\s+/g, "");
const srcExe = join(root, "src-tauri", "target", "release", "videodivider.exe");

if (!existsSync(srcExe)) {
  console.error(`Arquivo não encontrado: ${srcExe}`);
  console.error("Execute antes: npm run tauri build");
  process.exit(1);
}

const outRoot = join(root, "dist-portable");
const folderName = `${productSafe}-${version}-windows-x64-portable`;
const outFolder = join(outRoot, folderName);
const destExe = join(outFolder, `${productSafe}.exe`);
const zipPath = join(outRoot, `${folderName}.zip`);

mkdirSync(outFolder, { recursive: true });
copyFileSync(srcExe, destExe);

if (existsSync(zipPath)) {
  unlinkSync(zipPath);
}

execFileSync("tar", ["-a", "-cf", `${folderName}.zip`, folderName], {
  cwd: outRoot,
  stdio: "inherit",
});

console.log(`\nPortátil pronto:`);
console.log(`  Pasta: ${outFolder}`);
console.log(`  ZIP:   ${zipPath}`);
