/**
 * 初期セットアップの道しるべ（P2-1、`docs/mobile-ui-plan.md`）のクライアント。
 *
 * `systemAdmin.ts` / `usersAdmin.ts` と同じ小さな専用クライアントの形
 * （`admin`のみ・単一の読み取り GET・`DataProvider` の CRUD 契約に載らない）。
 * Tauri は `invoke('setup_status_get')`、LAN ブラウザは `fetch('/api/setup-status')`
 * で、どちらも同じサービス層（`admin_template_core::setup::setup_status`）に
 * 当たる（conventions §1）。
 *
 * ## デモモードでは `null`
 *
 * ブラウザ単体のデモ（InMemory provider）には裏の DB が無く、`setup_status`
 * を評価できない。他の Admin クライアント（`isXxxAvailable()` + throw）とは
 * 違い、ここは**呼び出し側が「非表示」として素直に扱える `null`** を返す
 * —— dashboard は「取得失敗時は何も表示しない」仕様なので、例外を1箇所で
 * 握りつぶすより、そもそも失敗しない形にした方が呼び出し側が単純になる。
 * デモの visual baseline（既存スクリーンショット）を動かさないための
 * 意図的な仕様でもある。
 */
import { invoke } from '@tauri-apps/api/core';
import { getAuthProvider, isProviderError, ProviderError, type ErrorBody } from '@banto/admin-core';
import { CSRF_HEADER, getBantoMode } from './setup';

/** `admin_template_core::setup::SetupStatus`（camelCase）。 */
export interface SetupStatus {
	issuerDone: boolean;
	ratesDone: boolean;
	customersDone: boolean;
	projectsDone: boolean;
	workLogsDone: boolean;
	allDone: boolean;
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

/** 他の `*Admin.ts` と同じ型ガード（spec §10/§11.1）。 */
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
	try {
		return (await invoke(cmd, args)) as T;
	} catch (err) {
		throw toProviderError(err);
	}
}

/** `usersAdmin.ts`/`systemAdmin.ts` と同じトークン参照。 */
function currentToken(): string | null {
	const auth = getAuthProvider() as { getToken?: () => string | null };
	return auth.getToken ? auth.getToken() : null;
}

async function httpGet<T>(path: string): Promise<T> {
	const headers: Record<string, string> = { ...CSRF_HEADER };
	const token = currentToken();
	if (token) headers.Authorization = `Bearer ${token}`;

	const response = await fetch(path, { method: 'GET', headers });
	if (!response.ok) {
		let body: unknown;
		try {
			body = await response.json();
		} catch {
			throw new ProviderError({
				kind: 'other',
				message: `${response.status} ${response.statusText}`
			});
		}
		if (isErrorBody(body)) throw new ProviderError(body);
		throw new ProviderError({
			kind: 'other',
			message: `${response.status} ${response.statusText}`
		});
	}
	return (await response.json()) as T;
}

/**
 * 初期セットアップの進捗を読む。**admin のみ**（発行者情報の有無を含むため
 * `issuer` と同じ床）。デモモードでは `null`（上の doc を参照）。
 *
 * 取得失敗（ネットワーク・権限不足等）も呼び出し側に例外を投げず `null` に
 * 畳む —— dashboard 側の「取得失敗時は何も表示しない」を、ここ1箇所で
 * 満たすため（呼び出し側に try/catch を書かせない）。
 */
export async function getSetupStatus(): Promise<SetupStatus | null> {
	if (getBantoMode() === 'demo') return null;
	try {
		if (getBantoMode() === 'tauri') return await invokeCommand<SetupStatus>('setup_status_get');
		return await httpGet<SetupStatus>('/api/setup-status');
	} catch {
		return null;
	}
}
