import { invoke } from "@tauri-apps/api/core";
import type { Tool } from "./types";

export interface FeishuProject {
  path: string;
  name: string;
}

export interface FeishuProjectGrant {
  path: string;
  read: boolean;
  create: boolean;
  edit: boolean;
  delete: boolean;
}

export interface FeishuBotSettings {
  name: string;
  appId: string;
  tenantKey: string;
  allowedUsers: string[];
  allowedGroups: string[];
  allowPrivate: boolean;
  engine: Tool;
  projects: FeishuProjectGrant[];
}

export interface FeishuBot extends FeishuBotSettings {
  id: string;
  hasSecret: boolean;
  status: "stopped" | "connecting" | "connected" | "error";
  error: string | null;
}

export interface FeishuBotInput extends FeishuBotSettings {
  id?: string;
  appSecret?: string;
}

export const listFeishuBots = () => invoke<FeishuBot[]>("feishu_list");
export const listFeishuProjects = () => invoke<FeishuProject[]>("feishu_projects");
export const saveFeishuBot = (input: FeishuBotInput) => invoke<FeishuBot>("feishu_save", { input });
export const removeFeishuBot = (id: string) => invoke<void>("feishu_remove", { id });
export const startFeishuBot = (id: string) => invoke<FeishuBot>("feishu_start", { id });
export const stopFeishuBot = (id: string) => invoke<FeishuBot>("feishu_stop", { id });
export const testFeishuBot = (id: string) => invoke<FeishuBot>("feishu_test", { id });
