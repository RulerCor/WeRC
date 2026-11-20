import { invoke, isTauri } from '@tauri-apps/api/core';
export interface Account { id: string; label: string; state: string; created_at: string }
export interface TokenInfo { id: string; label: string; account_ids: string[]; scopes: string[]; created_at: string; expires_at: string; revoked: boolean }
export interface IssuedToken { info: TokenInfo; secret: string }
export interface Capability { id: string; evidence: string; availability: string; reason: string; execution_mode: string | null; requires_foreground: boolean; requires_user_action: boolean }
export interface ClientInstallation { id: string; executable_path: string; version: string; architecture: string; product_name: string; sha256: string; process_ids: number[]; sources: string[]; compatibility: string; reason: string }
export interface ClientBinding { account_id: string; installation: ClientInstallation; bound_at: string; checked_at: string; validation: string }
export interface KeyStatus { account_id: string; protected: boolean; verified: boolean; verified_databases: number; verification_error: string | null; updated_at: string | null }
export interface DiscoveryReport { installations: ClientInstallation[]; warnings: string[] }
export interface DatabaseFile { relative_path: string; category: string; size: number; wal_present: boolean }
export interface DataSourceCandidate { id: string; account_directory: string; directory_name: string; layout: string; identity_verified: boolean; databases: DatabaseFile[]; has_media_directory: boolean }
export interface DataSourceCatalog { sources: DataSourceCandidate[]; warnings: string[]; roots_checked: number }
export interface AudioInfo { format: string; sample_rate: number; channels: number; bits_per_sample: number; frames: number; duration_ms: number; peak: number; rms: number }
export interface FileAsset {id:string;account_id:string;sha256:string;size:number;created_at:string;expires_at:string}
export interface FilePreview {asset:FileAsset;file_name:string}
export interface ImageAsset { id: string; account_id: string; sha256: string; source_sha256: string; size: number; image: {width: number; height: number}; created_at: string; expires_at: string }
export interface AudioAsset { id: string; account_id: string; sha256: string; size: number; audio: AudioInfo; created_at: string; expires_at: string }
export interface Operation { id: string; account_id: string; creator_id: string; kind: string; asset_id: string; state: string; result: { sent: boolean; execution: string; audio: AudioInfo } | null; error_code: string | null; created_at: string; updated_at: string }
export interface HistorySource { id: string; account_id: string; source_sha256: string; schema: string; encryption: string; message_count: number; contact_count: number; session_count: number; imported_at: string; real_wechat_verified: boolean }
export interface HistoryMessage { id: string; conversation_id: string; sender_id: string | null; server_id: string; timestamp: number; sequence: number; message_type: number; text: string | null; content_status: string }
export interface HistoryContact { username: string; nickname: string | null; remark: string | null; alias: string | null }
export interface HistorySession { username: string; summary: string | null; timestamp: number; unread_count: number | null }
export interface MessagePage { data: HistoryMessage[]; next_cursor: string | null; source_ids: string[]; data_mode: string }
export interface CurrentConnection { account_id: string | null; revision: string; selection_required: boolean; identity_verified: boolean }
export interface Bootstrap { current_connection: CurrentConnection; status: { product: string; version: string; stage: string; api_enabled: boolean; real_wechat_verified: boolean }; accounts: Account[]; tokens: TokenInfo[]; capabilities: Capability[]; api_address: string | null; bindings: ClientBinding[]; key_statuses: KeyStatus[]; audio_assets: AudioAsset[]; image_assets: ImageAsset[]; file_assets:FileAsset[]; operations: Operation[]; history_sources: HistorySource[] }
export const desktopAvailable = isTauri();
export async function command<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  if (!desktopAvailable) throw new Error('当前为界面预览，未连接 Tauri 本机核心；不能执行管理操作。');
  try { return await invoke<T>(name, args); }
  catch (error) {
    if (typeof error === 'object' && error && 'message' in error) {
      const code = 'code' in error ? String(error.code) : undefined;
      throw Object.assign(new Error(`${code ? `${code} / ` : ''}${String(error.message)}`), {code});
    }
    throw new Error(String(error));
  }
}

// Only fixed error kinds cross IPC; never send exception text, URLs or user content.
if (desktopAvailable) {
  window.addEventListener("error", () => { void invoke("diagnostic_frontend_error", {code:"FRONTEND_ERROR"}).catch(()=>{}); });
  window.addEventListener("unhandledrejection", () => { void invoke("diagnostic_frontend_error", {code:"FRONTEND_UNHANDLED_REJECTION"}).catch(()=>{}); });
}
