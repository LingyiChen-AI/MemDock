import {
  Atom, Braces, CodeXml, Container, Database, File, FileArchive, FileImage, FileText,
  GitBranch, LockKeyhole, Package, Settings2, Terminal, type LucideIcon,
} from "lucide-react";
import { cn } from "@/lib/utils";

type FileIcon = { icon: LucideIcon; color: string } | { label: string; color: string };

const json = { icon: Braces, color: "text-yellow-300" };
const config = { icon: Settings2, color: "text-slate-300" };
const shell = { icon: Terminal, color: "text-emerald-300" };
const react = { icon: Atom, color: "text-cyan-300" };
const image = { icon: FileImage, color: "text-fuchsia-300" };
const text = { icon: FileText, color: "text-slate-300" };
const lock = { icon: LockKeyhole, color: "text-amber-300" };
const packageFile = { icon: Package, color: "text-lime-300" };

const names = new Map<string, FileIcon>([
  ["package.json", packageFile],
  ["cargo.toml", packageFile],
  ["pyproject.toml", packageFile],
  ["go.mod", packageFile],
  ["package-lock.json", lock],
  ["pnpm-lock.yaml", lock],
  ["yarn.lock", lock],
  ["bun.lockb", lock],
  ["go.sum", lock],
  ["dockerfile", { icon: Container, color: "text-sky-300" }],
  ["makefile", shell],
  ["license", text],
  ["readme", text],
]);

const extensions = new Map<string, FileIcon>([
  ["ts", { label: "TS", color: "text-blue-300" }],
  ["mts", { label: "TS", color: "text-blue-300" }],
  ["cts", { label: "TS", color: "text-blue-300" }],
  ["js", { label: "JS", color: "text-yellow-300" }],
  ["mjs", { label: "JS", color: "text-yellow-300" }],
  ["cjs", { label: "JS", color: "text-yellow-300" }],
  ["tsx", react],
  ["jsx", react],
  ["vue", { label: "V", color: "text-emerald-300" }],
  ["svelte", { label: "S", color: "text-orange-300" }],
  ["html", { icon: CodeXml, color: "text-orange-300" }],
  ["htm", { icon: CodeXml, color: "text-orange-300" }],
  ["css", { label: "#", color: "text-violet-300" }],
  ["scss", { label: "SC", color: "text-pink-300" }],
  ["less", { label: "LS", color: "text-indigo-300" }],
  ["rs", { label: "Rs", color: "text-orange-300" }],
  ["py", { label: "Py", color: "text-sky-300" }],
  ["go", { label: "Go", color: "text-cyan-300" }],
  ["java", { label: "J", color: "text-orange-300" }],
  ["kt", { label: "Kt", color: "text-violet-300" }],
  ["cs", { label: "C#", color: "text-violet-300" }],
  ["c", { label: "C", color: "text-blue-300" }],
  ["h", { label: "H", color: "text-blue-300" }],
  ["cpp", { label: "C+", color: "text-blue-300" }],
  ["hpp", { label: "H+", color: "text-blue-300" }],
  ["rb", { label: "Rb", color: "text-rose-300" }],
  ["php", { label: "PHP", color: "text-indigo-300" }],
  ["md", { label: "M↓", color: "text-sky-300" }],
  ["mdx", { label: "M↓", color: "text-yellow-300" }],
  ["markdown", { label: "M↓", color: "text-sky-300" }],
  ["txt", text],
  ["json", json],
  ["jsonc", json],
  ["yaml", config],
  ["yml", config],
  ["toml", config],
  ["ini", config],
  ["lock", lock],
  ["sh", shell],
  ["bash", shell],
  ["zsh", shell],
  ["ps1", shell],
  ["cmd", shell],
  ["bat", shell],
  ["sql", { icon: Database, color: "text-rose-300" }],
  ["sqlite", { icon: Database, color: "text-rose-300" }],
  ["svg", image],
  ["png", image],
  ["jpg", image],
  ["jpeg", image],
  ["gif", image],
  ["webp", image],
  ["ico", image],
  ["zip", { icon: FileArchive, color: "text-amber-300" }],
]);

function fileIcon(name: string): FileIcon {
  const lower = name.toLowerCase();
  if (/^\.git(?:ignore|attributes|modules|keep)$/.test(lower)) return { icon: GitBranch, color: "text-orange-300" };
  if (lower === ".env" || lower.startsWith(".env.")) return config;
  const dot = lower.lastIndexOf(".");
  return names.get(lower) ?? (dot >= 0 ? extensions.get(lower.slice(dot + 1)) : undefined) ?? { icon: File, color: "text-slate-400" };
}

export function WorkspaceFileIcon({ name, className }: { name: string; className?: string }) {
  const descriptor = fileIcon(name);
  const style = cn("size-3.5 shrink-0", descriptor.color, className);
  if ("label" in descriptor) {
    return <span aria-hidden="true" className={cn("inline-grid place-items-center font-mono text-[8px] leading-none font-bold", style)}>{descriptor.label}</span>;
  }
  const Icon = descriptor.icon;
  return <Icon aria-hidden="true" className={style} />;
}
