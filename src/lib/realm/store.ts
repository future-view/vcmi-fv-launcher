import { useSyncExternalStore } from "react";
import {
  activateRealm,
  createRealm,
  deactivateRealm,
  deleteRealm,
  getActiveRealm,
  listRealms,
  setRealmMapsBlocked,
  updateRealm,
} from "@/lib/tauri";
import type { Realm, RealmList } from "./types";

/**
 * 领域状态 store：纯 TS 逻辑层（不依赖 React），通过 `useSyncExternalStore` 暴露给 UI。
 *
 * ⚠️ 关键：`getSnapshot` 必须返回**不可变快照**——任何写入都替换整个 `state` 引用，
 * 否则 React 用 `Object.is` 比对会判定「无变化」而跳过重渲染（见 working memory 的 React 陷阱）。
 */
interface RealmState {
  realms: Realm[];
  activeRealm: string | null;
  loaded: boolean;
}

let state: RealmState = { realms: [], activeRealm: null, loaded: false };
const listeners = new Set<() => void>();

function emit() {
  for (const l of listeners) l();
}

/** 以不可变替换方式更新状态，确保订阅者能感知变化。 */
function setState(next: RealmState) {
  state = next;
  emit();
}

function subscribe(cb: () => void): () => void {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}

function getSnapshot(): RealmState {
  return state;
}

/** 从后端拉取最新领域列表 + 激活态。首次进入领域页或任何领域变更后调用。 */
export async function refreshRealms(): Promise<void> {
  const list = await listRealms();
  setState({ realms: list.realms, activeRealm: list.activeRealm, loaded: true });
}

/** 读取当前激活态（不触发整表刷新，轻量）。 */
export async function refreshActiveRealm(): Promise<void> {
  const id = await getActiveRealm();
  setState({ ...state, activeRealm: id });
}

export async function createRealmSafe(data: unknown): Promise<Realm | null> {
  const r = await createRealm(data);
  await refreshRealms();
  return r;
}

export async function updateRealmSafe(realmId: string, data: unknown): Promise<void> {
  await updateRealm(realmId, data);
  await refreshRealms();
}

export async function deleteRealmSafe(realmId: string): Promise<void> {
  await deleteRealm(realmId);
  await refreshRealms();
}

export async function activateRealmSafe(realmId: string): Promise<boolean> {
  const res = await activateRealm(realmId);
  if (!res.ok) return false;
  await refreshRealms();
  return true;
}

export async function deactivateRealmSafe(): Promise<boolean> {
  const res = await deactivateRealm();
  if (!res.ok) return false;
  await refreshRealms();
  return true;
}

export async function setRealmMapsBlockedSafe(realmId: string, blocked: boolean): Promise<boolean> {
  const res = await setRealmMapsBlocked(realmId, blocked);
  if (!res.ok) return false;
  await refreshRealms();
  return true;
}

/** React 订阅钩子：返回 { realms, activeRealm, loaded } 不可变快照。 */
export function useRealms(): RealmState {
  return useSyncExternalStore(subscribe, getSnapshot);
}

/** 便捷：按 id 取领域 */
export function getRealmById(id: string): Realm | undefined {
  return state.realms.find((r) => r.id === id);
}
