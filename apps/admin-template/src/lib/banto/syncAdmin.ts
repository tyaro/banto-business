/**
 * デバイス間同期のクライアント（Phase 8、`docs/domain/sync.md` 11節）。
 *
 * ## なぜ Tauri 経路しか無いのか
 *
 * 同期は **話しかける側**（スマホのアプリ）でしか成立しない。相手（PC）の
 * 受け口は `/api/sync/*` で、そちらは既にある —— 同じ操作を LAN ブラウザ
 * からも呼べるようにすると、「ブラウザを開いている PC が、別の PC へ同期
 * しに行く」という誰も要求していない経路が増える。
 *
 * パスワードをアプリのメモリにだけ置く（`docs/domain/sync.md` 11.9）都合
 * からも、プロセスの外へ出せない。`invoke` だけを持つ。
 *
 * `usersAdmin.ts` / `invoicesAdmin.ts` と同じく、パッケージ側の DataProvider
 * 契約を曲げずに小さな専用クライアントを置く（conventions §4/§5）。
 */
import { invoke } from '@tauri-apps/api/core';
import { isProviderError, ProviderError, type ErrorBody } from '@banto/admin-core';
import { getBantoMode } from './setup';

/** `SyncSettings`（`src-tauri/src/lib.rs`）。**パスワードは含まない。** */
export interface SyncSettings {
	deviceId: number;
	peerUrl: string;
	peerUsername: string;
	/** 今のアプリの寿命の間にパスワードを入力済みか。 */
	hasPassword: boolean;
	/** 採番レンジを持つ表に行が在るか（＝デバイス番号を変えられない）。 */
	hasRows: boolean;
	openConflicts: number;
	lastSyncedAt: string | null;
}

export interface SyncSettingsInput {
	deviceId: number;
	peerUrl: string;
	peerUsername: string;
}

/** `admin_template_core::sync::client::SyncOutcome`。 */
export interface SyncOutcome {
	deviceId: number;
	peerDeviceId: number;
	pulledApplied: number;
	pulledUnchanged: number;
	pushedApplied: number;
	pushedUnchanged: number;
	conflictsDetected: number;
	openConflicts: number;
}

export const DESKTOP_ONLY_MESSAGE = 'この画面はアプリでのみ利用できます';

/** アプリ（Tauri）か。LAN ブラウザとデモは false。 */
export function isSyncAvailable(): boolean {
	return getBantoMode() === 'tauri';
}

const ERROR_KINDS = new Set([
	'not_found',
	'validation',
	'bad_request',
	'unauthorized',
	'forbidden',
	'storage',
	'other'
]);

function isErrorBody(value: unknown): value is ErrorBody {
	if (typeof value !== 'object' || value === null) return false;
	const kind = (value as { kind?: unknown }).kind;
	return typeof kind === 'string' && ERROR_KINDS.has(kind);
}

function toProviderError(err: unknown): ProviderError {
	if (isProviderError(err)) return err;
	if (isErrorBody(err)) return new ProviderError(err);
	const message = err instanceof Error ? err.message : String(err);
	return new ProviderError({ kind: 'other', message });
}

async function invokeCommand<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
	if (!isSyncAvailable()) {
		throw new ProviderError({ kind: 'other', message: DESKTOP_ONLY_MESSAGE });
	}
	try {
		return (await invoke(cmd, args)) as T;
	} catch (err) {
		throw toProviderError(err);
	}
}

export async function getSyncSettings(): Promise<SyncSettings> {
	return invokeCommand<SyncSettings>('sync_settings_get');
}

export async function applySyncSettings(input: SyncSettingsInput): Promise<SyncSettings> {
	return invokeCommand<SyncSettings>('sync_settings_apply', { input });
}

/**
 * 同期を1回実行する。
 *
 * `password` を渡すとアプリ側がメモリに控え、次からは省略できる
 * （`SyncSettings.hasPassword` が true になる）。**保存はされない** ——
 * アプリを終了すれば消える。
 */
export async function runSync(password?: string): Promise<SyncOutcome> {
	return invokeCommand<SyncOutcome>('sync_run', password ? { password } : {});
}
